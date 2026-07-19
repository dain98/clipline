//! Safe wrappers for the small Win32 surface owned by the application shell.

use std::ffi::c_void;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, FILETIME, HANDLE};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProcessIdentity {
    process_id: u32,
    creation_time: u64,
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

pub fn current_process_is_elevated() -> Result<bool, String> {
    process_is_elevated(std::process::id())
}

pub fn process_is_elevated(process_id: u32) -> Result<bool, String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return Err(last_error(format!("open process {process_id}")));
    }
    let process = OwnedHandle(process);

    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_error(format!("open process token {process_id}")));
    }
    let token = OwnedHandle(token);

    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0u32;
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast::<c_void>(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    } == 0
    {
        return Err(last_error(format!("query process elevation {process_id}")));
    }
    if returned < std::mem::size_of::<TOKEN_ELEVATION>() as u32 {
        return Err(format!(
            "query process elevation {process_id}: Windows returned {returned} bytes"
        ));
    }
    Ok(elevation.TokenIsElevated != 0)
}

pub fn process_instance_id(process_id: u32) -> Result<String, String> {
    let identity = query_process_identity(process_id)?;
    Ok(format!(
        "{}:{}",
        identity.process_id, identity.creation_time
    ))
}

fn query_process_identity(process_id: u32) -> Result<ProcessIdentity, String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return Err(last_error(format!("open process {process_id}")));
    }
    let process = OwnedHandle(process);
    process_identity_from_handle(process_id, process.0)
}

fn process_identity_from_handle(
    process_id: u32,
    process: HANDLE,
) -> Result<ProcessIdentity, String> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    if unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) } == 0 {
        return Err(last_error(format!(
            "query process creation time {process_id}"
        )));
    }
    Ok(ProcessIdentity {
        process_id,
        creation_time: (u64::from(creation.dwHighDateTime) << 32)
            | u64::from(creation.dwLowDateTime),
    })
}

fn last_error(context: String) -> String {
    let code = unsafe { GetLastError() };
    format!("{context}: Windows error {code}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_elevation_is_queryable() {
        current_process_is_elevated().expect("query this test process token");
    }

    #[test]
    fn current_process_instance_is_queryable() {
        let identity = query_process_identity(std::process::id())
            .expect("query this test process creation time");
        assert_eq!(identity.process_id, std::process::id());
        assert_ne!(identity.creation_time, 0);
    }
}
