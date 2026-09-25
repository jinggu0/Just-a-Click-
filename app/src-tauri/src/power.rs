//! Keeps Windows awake while recording. System power settings stay unchanged.
pub const ES_CONTINUOUS: u32 = 0x8000_0000;
pub const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
pub const ES_DISPLAY_REQUIRED: u32 = 0x0000_0002;
pub const RECORDING_STATE: u32 = ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED;

/// Holds the sleep request for as long as it lives; dropping it restores normal sleep.
pub struct KeepAwake {
    set_state: fn(u32) -> u32,
    accepted: bool,
}

impl KeepAwake {
    pub fn new() -> Self {
        Self::with(set_thread_execution_state)
    }

    pub fn with(set_state: fn(u32) -> u32) -> Self {
        let accepted = set_state(RECORDING_STATE) != 0;
        Self {
            set_state,
            accepted,
        }
    }

    /// Whether Windows took the request. `SetThreadExecutionState` answers with the
    /// previous state and returns zero only when the request was refused.
    pub fn accepted(&self) -> bool {
        self.accepted
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        (self.set_state)(ES_CONTINUOUS);
    }
}

#[cfg(windows)]
fn set_thread_execution_state(flags: u32) -> u32 {
    use windows::Win32::System::Power::{SetThreadExecutionState, EXECUTION_STATE};
    unsafe { SetThreadExecutionState(EXECUTION_STATE(flags)).0 }
}

#[cfg(not(windows))]
fn set_thread_execution_state(_flags: u32) -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static CALLS: AtomicU32 = AtomicU32::new(0);
    static LAST: AtomicU32 = AtomicU32::new(0);

    fn record(flags: u32) -> u32 {
        CALLS.fetch_add(1, Ordering::SeqCst);
        LAST.store(flags, Ordering::SeqCst);
        0
    }

    #[test]
    fn requests_sleep_block_and_restores_on_drop() {
        CALLS.store(0, Ordering::SeqCst);
        {
            let guard = KeepAwake::with(record);
            assert_eq!(CALLS.load(Ordering::SeqCst), 1);
            assert_eq!(LAST.load(Ordering::SeqCst), 0x8000_0003);
            assert!(!guard.accepted(), "a zero answer means the request was refused");
        }
        assert_eq!(CALLS.load(Ordering::SeqCst), 2);
        assert_eq!(LAST.load(Ordering::SeqCst), ES_CONTINUOUS);
    }

    #[test]
    #[cfg(windows)]
    fn windows_takes_the_recording_request() {
        let guard = KeepAwake::new();
        assert!(
            guard.accepted(),
            "SetThreadExecutionState refused the recording request"
        );
    }
}
