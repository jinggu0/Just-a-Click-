//! Child processes that cannot outlive the app.
//!
//! Windows kills every process in a job object when the last handle to it closes, so the
//! inference runtimes go away even when the app is force-closed and cannot clean up.
use std::ffi::c_void;
use std::os::windows::io::AsRawHandle;
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
    JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

const POLL: Duration = Duration::from_millis(50);

/// A job object every inference child is assigned to.
pub struct ProcessGroup {
    job: HANDLE,
}

impl ProcessGroup {
    pub fn new() -> Result<Self, String> {
        let job = unsafe { CreateJobObjectW(None, windows::core::PCWSTR::null()) }
            .map_err(|error| format!("job object failed: {error}"))?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(|error| format!("job object limits failed: {error}"))?;
        Ok(Self { job })
    }

    /// Starts the command and puts it in the group before it can outlive the app.
    pub fn spawn(&self, command: &mut Command) -> Result<Child, String> {
        let child = command.spawn().map_err(|error| format!("start failed: {error}"))?;
        let handle = HANDLE(child.as_raw_handle() as *mut c_void);
        unsafe { AssignProcessToJobObject(self.job, handle) }
            .map_err(|error| format!("could not group the child: {error}"))?;
        Ok(child)
    }
}

// A job object handle is process-wide and the Win32 calls on it are thread-safe, so the
// group can be shared with the worker threads that start the children.
unsafe impl Send for ProcessGroup {}
unsafe impl Sync for ProcessGroup {}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.job) };
    }
}

/// Waits for the child, returning None when it is still running after the deadline.
pub fn wait_for_exit(child: &mut Child, grace: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + grace;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            Ok(None) => return None,
            Err(_) => return None,
        }
    }
}

/// Ends a child and waits for it. Windows has no graceful signal, so this terminates.
pub fn stop(child: &mut Child, grace: Duration) -> Result<(), String> {
    if child.try_wait().map_err(|error| error.to_string())?.is_some() {
        return Ok(());
    }
    child.kill().map_err(|error| format!("stop failed: {error}"))?;
    match wait_for_exit(child, grace) {
        Some(_) => Ok(()),
        None => Err(format!("the child was still running after {:.0}s", grace.as_secs_f64())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sleeper() -> Command {
        let mut command = Command::new("cmd");
        command.args(["/c", "ping -n 30 127.0.0.1 > nul"]);
        command
    }

    #[test]
    fn a_child_dies_when_the_group_is_dropped() {
        let group = ProcessGroup::new().expect("job object");
        let mut child = group.spawn(&mut sleeper()).expect("spawn");
        assert!(child.try_wait().expect("try_wait").is_none(), "child should still run");
        drop(group);
        assert!(
            wait_for_exit(&mut child, Duration::from_secs(5)).is_some(),
            "the job object should have killed the child"
        );
    }

    #[test]
    fn stop_ends_a_running_child() {
        let group = ProcessGroup::new().expect("job object");
        let mut child = group.spawn(&mut sleeper()).expect("spawn");
        stop(&mut child, Duration::from_secs(5)).expect("stop");
        assert!(child.try_wait().expect("try_wait").is_some(), "child should be gone");
    }
}
