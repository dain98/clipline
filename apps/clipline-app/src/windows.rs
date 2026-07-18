//! Safe wrappers for the small Win32 surface owned by the application shell.

use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, HANDLE};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, OpenProcessToken, WaitForSingleObject, INFINITE, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const ELEVATED_AFTER_ARGUMENT: &str = "--clipline-elevated-after";
const PROCESS_SYNCHRONIZE: u32 = 0x0010_0000;

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

/// Launch the current executable through the UAC `runas` verb. The elevated
/// child waits for `parent_process_id` before starting Tauri, so the existing
/// single-instance owner is gone before the replacement claims it.
pub fn launch_elevated_after(parent_process_id: u32) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|e| format!("locate Clipline executable for administrator restart: {e}"))?;
    launch_elevated_executable(&executable, parent_process_id)
}

fn launch_elevated_executable(executable: &Path, parent_process_id: u32) -> Result<(), String> {
    let verb = wide("runas");
    let executable = wide_os(executable.as_os_str());
    let parameters = wide(&elevation_restart_parameters(parent_process_id));
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            executable.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        return Err(
            "Administrator restart was cancelled or denied; Clipline is still running normally."
                .into(),
        );
    }
    Ok(())
}

pub fn wait_for_elevation_parent_from_args() -> Result<(), String> {
    let Some(parent_process_id) = elevation_parent_from_args(std::env::args())? else {
        return Ok(());
    };
    wait_for_process_exit(parent_process_id)
}

fn elevation_parent_from_args<I>(args: I) -> Result<Option<u32>, String>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        if argument.as_ref() != ELEVATED_AFTER_ARGUMENT {
            continue;
        }
        let raw = args
            .next()
            .ok_or_else(|| format!("{ELEVATED_AFTER_ARGUMENT} requires a process id"))?;
        let process_id = raw
            .as_ref()
            .parse::<u32>()
            .map_err(|_| format!("invalid parent process id: {}", raw.as_ref()))?;
        return Ok(Some(process_id));
    }
    Ok(None)
}

fn wait_for_process_exit(process_id: u32) -> Result<(), String> {
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, process_id) };
    if process.is_null() {
        return parent_open_failure(process_id, unsafe { GetLastError() });
    }
    let process = OwnedHandle(process);
    let result = unsafe { WaitForSingleObject(process.0, INFINITE) };
    if result == u32::MAX {
        return Err(last_error(format!(
            "wait for Clipline process {process_id}"
        )));
    }
    Ok(())
}

fn parent_open_failure(process_id: u32, error_code: u32) -> Result<(), String> {
    if error_code == ERROR_INVALID_PARAMETER {
        // The parent completed between ShellExecute returning and this child
        // reaching main, leaving no process to wait for.
        Ok(())
    } else {
        Err(format!(
            "open Clipline process {process_id} for handoff: Windows error {error_code}"
        ))
    }
}

fn elevation_restart_parameters(parent_process_id: u32) -> String {
    format!("{ELEVATED_AFTER_ARGUMENT} {parent_process_id}")
}

fn wide(value: &str) -> Vec<u16> {
    wide_os(OsStr::new(value))
}

fn wide_os(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn last_error(context: String) -> String {
    let code = unsafe { GetLastError() };
    format!("{context}: Windows error {code}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevated_restart_argument_round_trips_parent_process() {
        let parameters = elevation_restart_parameters(4242);
        let args = ["clipline-app.exe", parameters.as_str()]
            .into_iter()
            .flat_map(str::split_whitespace);
        assert_eq!(elevation_parent_from_args(args).unwrap(), Some(4242));
    }

    #[test]
    fn ordinary_launch_has_no_elevation_parent() {
        assert_eq!(
            elevation_parent_from_args(["clipline-app.exe", "--autostart"]).unwrap(),
            None
        );
    }

    #[test]
    fn current_process_elevation_is_queryable() {
        current_process_is_elevated().expect("query this test process token");
    }

    #[test]
    fn parent_open_failure_only_ignores_a_gone_process() {
        assert!(parent_open_failure(
            4242,
            windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER
        )
        .is_ok());
        assert!(
            parent_open_failure(4242, windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED).is_err()
        );
    }
}
