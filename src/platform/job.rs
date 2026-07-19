//! Tying child processes to the lifetime of this one.
//!
//! A database child must never outlive the app that started it: an orphan keeps
//! the data directory locked, so the next launch fails.
//!
//! Each platform offers a different guarantee, and only two of the three are
//! airtight:
//!
//! - **Windows** — a job object with `KILL_ON_JOB_CLOSE`. When this process
//!   ends for *any* reason, including `TerminateProcess` or Task Manager, the
//!   kernel closes the last handle to the job and kills everything in it.
//! - **Linux** — `PR_SET_PDEATHSIG`, armed in the child (see `desktop::db`).
//! - **macOS** — neither exists. Orphan prevention degrades to the explicit
//!   shutdown path plus a sweep at the next launch.

/// A job that kills its members when dropped. Inert off Windows.
#[derive(Debug)]
pub struct ProcessGroup {
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}

impl ProcessGroup {
    /// Create a group whose members die with this process.
    ///
    /// Returns `Ok(None)` where the platform has no equivalent, so callers can
    /// treat "unsupported" differently from "failed".
    #[cfg(windows)]
    pub fn new() -> Result<Option<Self>, String> {
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        // The handle must not be inheritable: a child holding a reference would
        // keep the job alive past our exit, defeating the whole mechanism.
        // SAFETY: null arguments request an unnamed job with default security.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(format!(
                "cannot create job object: {}",
                std::io::Error::last_os_error()
            ));
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `info` is a correctly sized, zero-initialised structure of the
        // class named by `JobObjectExtendedLimitInformation`.
        let ok = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err(format!(
                "cannot configure job object: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Some(ProcessGroup { job }))
    }

    #[cfg(not(windows))]
    pub fn new() -> Result<Option<Self>, String> {
        Ok(None)
    }

    /// Put an already-spawned child into the group.
    #[cfg(windows)]
    pub fn adopt(&self, child: &std::process::Child) -> Result<(), String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

        let handle: HANDLE = child.as_raw_handle() as _;
        // SAFETY: both handles are live — the job for our lifetime, the process
        // because `child` has not been reaped.
        let ok = unsafe { AssignProcessToJobObject(self.job, handle) };
        if ok == 0 {
            return Err(format!(
                "cannot assign process to job: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn adopt(&self, _child: &std::process::Child) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        // Closing the last handle is what triggers KILL_ON_JOB_CLOSE.
        // SAFETY: `job` was created by us and is closed exactly once.
        unsafe {
            CloseHandle(self.job);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_a_no_op_where_unsupported() {
        // Off Windows this must report "unsupported", not "failed" — the caller
        // uses the distinction to decide whether to warn.
        let group = ProcessGroup::new().expect("must not error");
        #[cfg(windows)]
        assert!(group.is_some());
        #[cfg(not(windows))]
        assert!(group.is_none(), "no job objects outside Windows");
    }
}
