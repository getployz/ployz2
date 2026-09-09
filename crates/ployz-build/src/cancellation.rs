//! Interrupt all phases of an admitted attempt, while allowing bounded cleanup.

use crate::BuildError;
use std::sync::{Arc, atomic::AtomicBool};

/// Own the signal registrations for one synchronous Build Attempt.
pub(crate) struct Cancellation {
    /// Set when the caller interrupts; cleanup commands ignore this flag.
    pub(crate) flag: Arc<AtomicBool>,
    signals: Vec<signal_hook::SigId>,
}

impl Cancellation {
    /// Observe SIGINT and SIGTERM until this attempt leaves scope.
    ///
    /// # Errors
    /// Fails when the operating system cannot register a signal handler.
    pub(crate) fn new() -> Result<Self, BuildError> {
        let mut cancellation = Self {
            flag: Arc::new(AtomicBool::new(false)),
            signals: Vec::new(),
        };
        for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
            cancellation.signals.push(
                signal_hook::flag::register(signal, Arc::clone(&cancellation.flag)).map_err(
                    |error| {
                        BuildError::Prerequisite(format!("install build cancellation: {error}"))
                    },
                )?,
            );
        }
        Ok(cancellation)
    }
}

impl Drop for Cancellation {
    fn drop(&mut self) {
        for signal in &self.signals {
            signal_hook::low_level::unregister(*signal);
        }
    }
}
