//! Machine-local FIFO admission. Permits follow execution, not the RPC task.
//!
//! The number of build slots is the Machine's build concurrency. It is read
//! from the record at each arrival and after each Machine update, so a changed
//! limit applies to queued Builds without restarting the daemon.

use ployz_build::{BuildError, HostPolicy};
use ployz_core::BuildConcurrency;
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};
use tokio::{
    sync::{AcquireError, OwnedSemaphorePermit, Semaphore},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

pub(crate) struct Runner {
    pub(super) policy: HostPolicy,
    active: Arc<Semaphore>,
    slots: Arc<Mutex<Slots>>,
    waiting: Arc<Semaphore>,
    pub(super) shutdown: CancellationToken,
}

/// `available + held - debt == size`. A lowered limit cannot take permits back
/// from running Builds, so it records debt and forgets them as they finish.
struct Slots {
    size: usize,
    debt: usize,
}

/// One admitted build slot. Dropping it frees the slot unless a lowered limit
/// is still owed.
pub(super) struct Slot {
    permit: Option<OwnedSemaphorePermit>,
    slots: Arc<Mutex<Slots>>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut slots = self.slots.lock().expect("build slot lock");
        if slots.debt > 0
            && let Some(permit) = self.permit.take()
        {
            slots.debt -= 1;
            permit.forget();
        }
    }
}

impl Runner {
    pub(crate) fn new(
        policy: HostPolicy,
        shutdown: CancellationToken,
    ) -> Result<Arc<Self>, BuildError> {
        policy.validate()?;
        Ok(Arc::new(Self {
            waiting: Arc::new(Semaphore::new(policy.queue_capacity)),
            active: Arc::new(Semaphore::new(1)),
            slots: Arc::new(Mutex::new(Slots { size: 1, debt: 0 })),
            policy,
            shutdown,
        }))
    }

    /// Set the number of simultaneous Builds. Raising it admits queued Builds
    /// in FIFO order; lowering it lets running Builds finish.
    pub(crate) fn resize(&self, limit: BuildConcurrency) {
        let limit = usize::from(limit);
        let mut slots = self.slots.lock().expect("build slot lock");
        if limit > slots.size {
            let grow = limit - slots.size;
            let repaid = grow.min(slots.debt);
            slots.debt -= repaid;
            self.active.add_permits(grow - repaid);
        } else {
            let shrink = slots.size - limit;
            slots.debt += shrink - self.active.forget_permits(shrink);
        }
        slots.size = limit;
    }

    pub(super) fn enter(
        &self,
        limit: BuildConcurrency,
    ) -> Result<
        Entry<impl Future<Output = Result<OwnedSemaphorePermit, AcquireError>> + Send>,
        QueueError,
    > {
        if self.shutdown.is_cancelled() {
            return Err(QueueError::Stopping);
        }
        self.resize(limit);
        let mut admission = Box::pin(self.active.clone().acquire_owned());
        // Enroll synchronously so a release or a slow progress consumer cannot
        // let a later arrival overtake this waiter before its first await.
        match admission
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(result) => result
                .map(|permit| Entry::Active(self.slot(permit)))
                .map_err(|_| QueueError::Stopping),
            Poll::Pending => {
                let capacity = self
                    .waiting
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| QueueError::Full)?;
                Ok(Entry::Waiting(Waiting {
                    _capacity: capacity,
                    admission,
                    expires: Instant::now() + self.policy.queue_timeout,
                }))
            }
        }
    }

    pub(super) async fn admit<F>(&self, waiting: Waiting<F>) -> Result<Slot, QueueError>
    where
        F: Future<Output = Result<OwnedSemaphorePermit, AcquireError>>,
    {
        tokio::select! {
            biased;
            () = tokio::time::sleep_until(waiting.expires) => Err(QueueError::Expired),
            result = waiting.admission => result.map(|permit| self.slot(permit)).map_err(|_| QueueError::Stopping),
        }
    }

    fn slot(&self, permit: OwnedSemaphorePermit) -> Slot {
        Slot {
            permit: Some(permit),
            slots: Arc::clone(&self.slots),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum QueueError {
    #[error("Build queue is full; execution was not attempted")]
    Full,
    #[error("Build queue timeout expired; execution was not attempted")]
    Expired,
    #[error("Build daemon is stopping; execution was not attempted")]
    Stopping,
}

pub(super) enum Entry<F> {
    Active(Slot),
    Waiting(Waiting<F>),
}

pub(super) struct Waiting<F> {
    _capacity: OwnedSemaphorePermit,
    admission: Pin<Box<F>>,
    expires: Instant,
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::poll;
    use std::{task::Poll, time::Duration};

    const ONE: BuildConcurrency = BuildConcurrency::ONE;

    fn active_within(runner: &Runner, limit: BuildConcurrency) -> Slot {
        let Entry::Active(slot) = runner.enter(limit).unwrap() else {
            panic!("expected admission")
        };
        slot
    }
    fn active(runner: &Runner) -> Slot {
        active_within(runner, ONE)
    }
    fn waiting_within(
        runner: &Runner,
        limit: BuildConcurrency,
    ) -> impl Future<Output = Result<Slot, QueueError>> {
        let Entry::Waiting(waiting) = runner.enter(limit).unwrap() else {
            panic!("expected waiting")
        };
        runner.admit(waiting)
    }
    fn waiting(runner: &Runner) -> impl Future<Output = Result<Slot, QueueError>> {
        waiting_within(runner, ONE)
    }
    fn limit(value: &str) -> BuildConcurrency {
        BuildConcurrency::parse(value).unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn build_concurrency_bounds_simultaneous_builds_and_follows_changes() {
        let runner = Runner::new(HostPolicy::default(), CancellationToken::new()).unwrap();
        let automatic = BuildConcurrency::automatic(false, Some(12_000_000_000));
        let first = active_within(&runner, automatic);
        let second = active_within(&runner, automatic);
        let third = active_within(&runner, automatic);
        let mut fourth = Box::pin(waiting_within(&runner, automatic));
        assert!(
            poll!(&mut fourth).is_pending(),
            "automatic 3 admitted a fourth"
        );

        // Raising the limit admits the queued Build without a new arrival.
        runner.resize(limit("4"));
        let Poll::Ready(Ok(fourth)) = poll!(&mut fourth) else {
            panic!("raised limit did not admit the queued Build")
        };

        // Lowering it lets running Builds finish and admits nothing until
        // fewer than the new limit remain.
        let mut fifth = Box::pin(waiting_within(&runner, limit("2")));
        drop(first);
        drop(second);
        assert!(poll!(&mut fifth).is_pending(), "lowered limit was exceeded");
        drop(third);
        let Poll::Ready(Ok(fifth)) = poll!(&mut fifth) else {
            panic!("Build was not admitted under the lowered limit")
        };
        assert!(matches!(runner.enter(limit("2")), Ok(Entry::Waiting(_))));
        drop((fourth, fifth));
        let _both = (
            active_within(&runner, limit("2")),
            active_within(&runner, limit("2")),
        );
    }

    #[tokio::test(start_paused = true)]
    async fn enrolled_waiter_cannot_be_overtaken_before_it_is_polled() {
        let runner = Runner::new(HostPolicy::default(), CancellationToken::new()).unwrap();
        let owner = active(&runner);
        let _first = runner.enter(ONE).unwrap();
        drop(owner);
        assert!(
            matches!(runner.enter(ONE), Ok(Entry::Waiting(_))),
            "new arrival overtook an enrolled waiter"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn fifo_bounds_expiry_and_cancelled_waiters_release_capacity() {
        let runner = Runner::new(HostPolicy::default(), CancellationToken::new()).unwrap();
        let owner = active(&runner);
        let mut entries = (0..8).map(|_| waiting(&runner)).collect::<Vec<_>>();
        assert!(matches!(runner.enter(ONE), Err(QueueError::Full)));
        let mut first = Box::pin(entries.remove(0));
        let mut second = Box::pin(entries.remove(0));
        assert!(poll!(&mut first).is_pending());
        assert!(poll!(&mut second).is_pending());
        drop(owner);
        assert!(poll!(&mut second).is_pending());
        let Poll::Ready(Ok(owner)) = poll!(&mut first) else {
            panic!("FIFO head did not get admission")
        };
        // Cancelling a queued future removes both its FIFO entry and capacity.
        drop(second);
        drop(entries);
        let mut expiring = Box::pin(waiting(&runner));
        assert!(poll!(&mut expiring).is_pending());
        tokio::time::advance(Duration::from_secs(599)).await;
        assert!(poll!(&mut expiring).is_pending());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(matches!(
            poll!(&mut expiring),
            Poll::Ready(Err(QueueError::Expired))
        ));
        assert!(matches!(runner.enter(ONE), Ok(Entry::Waiting(_))));
        drop(owner);
        drop(active(&runner));
    }
}
