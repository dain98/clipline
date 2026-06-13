use std::collections::{HashMap, HashSet};
use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_VM_READ,
};

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct MemoryStatus {
    pub working_set_bytes: u64,
}

pub fn current_process_tree_memory() -> Result<MemoryStatus, String> {
    let current_pid = unsafe { GetCurrentProcessId() };
    let mut total = process_working_set_bytes(unsafe { GetCurrentProcess() })?;

    for pid in child_process_ids(current_pid)? {
        if let Some(bytes) = process_working_set_for_pid(pid) {
            total = total.saturating_add(bytes);
        }
    }

    Ok(MemoryStatus {
        working_set_bytes: total,
    })
}

fn child_process_ids(root_pid: u32) -> Result<Vec<u32>, String> {
    let snapshot = Snapshot::new()?;
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..unsafe { zeroed() }
    };
    let mut children: HashMap<u32, Vec<ProcessEntry>> = HashMap::new();

    let mut ok = unsafe { Process32FirstW(snapshot.handle, &mut entry) };
    while ok != 0 {
        children
            .entry(entry.th32ParentProcessID)
            .or_default()
            .push(ProcessEntry {
                pid: entry.th32ProcessID,
                name: process_name(&entry),
            });
        ok = unsafe { Process32NextW(snapshot.handle, &mut entry) };
    }

    let mut out = Vec::new();
    let mut seen = HashSet::from([root_pid]);
    let mut stack = children.remove(&root_pid).unwrap_or_default();
    while let Some(process) = stack.pop() {
        if !seen.insert(process.pid) {
            continue;
        }
        if process.name != "conhost.exe" {
            out.push(process.pid);
        }
        if let Some(grandchildren) = children.remove(&process.pid) {
            stack.extend(grandchildren);
        }
    }
    Ok(out)
}

struct ProcessEntry {
    pid: u32,
    name: String,
}

fn process_name(entry: &PROCESSENTRY32W) -> String {
    let len = entry
        .szExeFile
        .iter()
        .position(|ch| *ch == 0)
        .unwrap_or(entry.szExeFile.len());
    String::from_utf16_lossy(&entry.szExeFile[..len]).to_ascii_lowercase()
}

fn process_working_set_for_pid(pid: u32) -> Option<u64> {
    let handle =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let result = process_working_set_bytes(handle).ok();
    unsafe {
        CloseHandle(handle);
    }
    result
}

fn process_working_set_bytes(handle: HANDLE) -> Result<u64, String> {
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..unsafe { zeroed() }
    };
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            handle,
            &mut counters,
            size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
    };
    if ok == 0 {
        return Err("could not read process memory counters".into());
    }
    Ok(counters.WorkingSetSize as u64)
}

struct Snapshot {
    handle: HANDLE,
}

impl Snapshot {
    fn new() -> Result<Self, String> {
        let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if handle == INVALID_HANDLE_VALUE {
            return Err("could not snapshot process list".into());
        }
        Ok(Self { handle })
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}
