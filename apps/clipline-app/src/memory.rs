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

    // This is a process-tree working set total, not private/unique memory.
    // Shared pages can be counted once per process, especially across WebView2
    // children; the UI uses it as a rough live footprint indicator.
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
    Ok(child_process_ids_from_entries(
        root_pid,
        &process_snapshot()?,
    ))
}

fn process_snapshot() -> Result<Vec<ProcessEntry>, String> {
    let snapshot = Snapshot::new()?;
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..unsafe { zeroed() }
    };
    let mut processes = Vec::new();

    let mut ok = unsafe { Process32FirstW(snapshot.handle, &mut entry) };
    while ok != 0 {
        processes.push(ProcessEntry {
            pid: entry.th32ProcessID,
            parent_pid: entry.th32ParentProcessID,
            name: process_name(&entry),
        });
        ok = unsafe { Process32NextW(snapshot.handle, &mut entry) };
    }

    Ok(processes)
}

fn child_process_ids_from_entries(root_pid: u32, entries: &[ProcessEntry]) -> Vec<u32> {
    let mut children: HashMap<u32, Vec<ProcessEntry>> = HashMap::new();
    for entry in entries {
        children
            .entry(entry.parent_pid)
            .or_default()
            .push(entry.clone());
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
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProcessEntry {
    pid: u32,
    parent_pid: u32,
    name: String,
}

fn process_name(entry: &PROCESSENTRY32W) -> String {
    process_name_from_wide(&entry.szExeFile)
}

fn process_name_from_wide(name: &[u16]) -> String {
    let len = name.iter().position(|ch| *ch == 0).unwrap_or(name.len());
    String::from_utf16_lossy(&name[..len]).to_ascii_lowercase()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, parent_pid: u32, name: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent_pid,
            name: name.to_string(),
        }
    }

    fn sorted(mut ids: Vec<u32>) -> Vec<u32> {
        ids.sort_unstable();
        ids
    }

    #[test]
    fn child_process_ids_walks_descendants_and_skips_conhost() {
        let ids = child_process_ids_from_entries(
            10,
            &[
                proc(20, 10, "msedgewebview2.exe"),
                proc(30, 20, "renderer.exe"),
                proc(40, 10, "conhost.exe"),
                proc(50, 40, "debug-child.exe"),
                proc(60, 99, "unrelated.exe"),
            ],
        );

        assert_eq!(sorted(ids), vec![20, 30, 50]);
    }

    #[test]
    fn child_process_ids_deduplicates_cycles() {
        let ids = child_process_ids_from_entries(
            1,
            &[
                proc(2, 1, "child.exe"),
                proc(3, 2, "grandchild.exe"),
                proc(2, 3, "reused-pid.exe"),
            ],
        );

        assert_eq!(sorted(ids), vec![2, 3]);
    }

    #[test]
    fn process_name_from_wide_stops_at_nul_and_lowercases() {
        let mut raw: Vec<u16> = "ClipLine.EXE".encode_utf16().collect();
        raw.push(0);
        raw.extend("ignored.exe".encode_utf16());

        assert_eq!(process_name_from_wide(&raw), "clipline.exe");
    }

    #[test]
    fn process_name_from_wide_accepts_unterminated_names() {
        let raw: Vec<u16> = "WebView2.EXE".encode_utf16().collect();

        assert_eq!(process_name_from_wide(&raw), "webview2.exe");
    }
}
