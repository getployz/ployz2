//! Prepared deployment execution and its owned progress stream.
use super::{ImageCleanup, PreparedDeploy, RunningDeploy, Session, closed, invalid_argument};
use crate::deploy::DeployPreview;
use ployz_core::{DeployEvent, DeployOutcome, ExecutionError, RpcError, RpcErrorCode};
use serde_json::Value;
use std::{ops::Deref, sync::atomic::Ordering};
use tokio::sync::{Mutex, mpsc};

impl PreparedDeploy {
    /// Private completed Build evidence for a later preparation; not runtime truth.
    #[must_use]
    pub fn build_receipts(
        &self,
    ) -> &std::collections::BTreeMap<ployz_core::ServiceName, super::preparation::BuildReceipt>
    {
        &self.build_receipts
    }

    /// Informational preview; execution remains bound to this prepared handle.
    #[must_use]
    pub fn preview(&self) -> &DeployPreview {
        self.preview.preview()
    }

    /// True when this preview planned no operations.
    #[must_use]
    pub fn noop(&self) -> bool {
        self.preview.noop()
    }

    /// Machines and repositories whose superseded build images Image Cleanup removes.
    #[must_use]
    pub fn prune_targets(&self) -> &[ployz_core::PruneTarget] {
        &self.prune_targets
    }

    /// Release an unconfirmed preparation and its retained images.
    pub fn close(&self) {
        let mut retained = self.retained.lock().expect("retained build lock");
        self.confirmed.store(true, Ordering::SeqCst);
        retained.take();
    }

    /// Execute these operations. Illegal after a previous confirm.
    ///
    /// # Errors
    ///
    /// Returns when this preview already confirmed, or when the session is closed.
    pub fn confirm(&self) -> Result<RunningDeploy, RpcError> {
        self.confirm_with_log_id(None, ImageCleanup::Auto)
    }

    /// Execute with caller-owned log correlation, without changing the planned configuration.
    /// Automatic Image Cleanup runs after the Outcome, whatever it is, and its report is
    /// the last event; it never changes the Outcome.
    /// # Errors
    /// Returns the same admission and session errors as confirm.
    pub fn confirm_with_log_id(
        &self,
        deployment_id: Option<ployz_core::DeploymentLogId>,
        cleanup: ImageCleanup,
    ) -> Result<RunningDeploy, RpcError> {
        let session = Session {
            inner: self.session.upgrade().ok_or_else(closed)?,
        };
        if session.inner.cancel.is_cancelled() {
            return Err(closed());
        }
        let mut retained = self.retained.lock().expect("retained build lock");
        if self.confirmed.swap(true, Ordering::SeqCst) {
            return Err(invalid_argument(
                "this Deploy Preview already confirmed".into(),
            ));
        }
        let cancel = session.inner.cancel.child_token();
        let (tx, rx) = mpsc::unbounded_channel();
        let mut client = session.client()?;
        client.deployment_id = deployment_id;
        let preview = self.preview.clone();
        let token = cancel.clone();
        let session_cancel = session.inner.cancel.clone();
        let retained = retained.take();
        let prune_targets = match cleanup {
            ImageCleanup::Auto => self.prune_targets.clone(),
            ImageCleanup::Manual => Vec::new(),
        };
        let join = tokio::spawn(async move {
            let _retained = retained;
            let events = tx.clone();
            let outcome = tokio::select! {
                biased;
                () = session_cancel.cancelled() => return Err(RpcError {
                    code: RpcErrorCode::Unavailable,
                    message: "session closed; in-flight Deploy outcome may be uncertain".into(),
                    details: Value::Null,
                }),
                outcome = client.confirm(&preview, &token, Some(tx)) => outcome,
            };
            if !prune_targets.is_empty() && !token.is_cancelled() {
                tokio::select! {
                    biased;
                    () = session_cancel.cancelled() => {}
                    report = crate::image::prune_images(&client, &prune_targets) => {
                        let _ = events.send(DeployEvent::ImagesPruned { report });
                    }
                }
            }
            Ok(outcome)
        });
        Ok(RunningDeploy {
            cancel,
            events: Mutex::new(Some(rx)),
            join: Mutex::new(Some(join)),
        })
    }
}

impl Deref for PreparedDeploy {
    type Target = DeployPreview;

    fn deref(&self) -> &Self::Target {
        &self.preview
    }
}

impl RunningDeploy {
    /// Cancel this Deploy. The outcome is a failed Deploy with `cancelled`.
    pub fn abort(&self) {
        self.cancel.cancel();
    }

    /// Next progress or outcome event, or `None` when the stream ended.
    pub async fn next(&self) -> Option<DeployEvent> {
        let mut guard = self.events.lock().await;
        let rx = guard.as_mut()?;
        if let Some(event) = rx.recv().await {
            return Some(event);
        }
        *guard = None;
        None
    }

    /// Wait for the Deploy Outcome. Progress events are still produced.
    ///
    /// # Errors
    /// Returns unavailable when session closure interrupts execution; mutations may have completed.
    pub async fn finished(&self) -> Result<DeployOutcome<ExecutionError>, RpcError> {
        let handle = self
            .join
            .lock()
            .await
            .take()
            .expect("deploy already finished");
        let outcome = handle.await.expect("deploy task joins");
        while self.next().await.is_some() {}
        outcome
    }
}
