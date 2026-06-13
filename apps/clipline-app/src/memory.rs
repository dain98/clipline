use std::collections::{HashMap, HashSet};
use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Memory::{
    VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_PRIVATE,
};
use windows_sys::Win32::System::ProcessStatus::{
    K32QueryWorkingSetEx, PSAPI_WORKING_SET_EX_INFORMATION,
};
use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_VM_READ,
};

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct MemoryStatus {
    pub private_working_set_bytes: u64,
}

pub fn current_process_tree_memory() -> Result<MemoryStatus, String> {
    let current_pid = unsafe { GetCurrentProcessId() };
    let page_size = page_size()?;
    let mut total = process_private_working_set_bytes(unsafe { GetCurrentProcess() }, page_size)?;

    for pid in child_process_ids(current_pid)? {
        if let Some(bytes) = process_private_working_set_for_pid(pid, page_size) {
            total = total.saturating_add(bytes);
        }
    }

    Ok(MemoryStatus {
        private_working_set_bytes: total,
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

fn process_private_working_set_for_pid(pid: u32, page_size: usize) -> Option<u64> {
    let handle =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let result = process_private_working_set_bytes(handle, page_size).ok();
    unsafe {
        CloseHandle(handle);
    }
    result
}

fn process_private_working_set_bytes(handle: HANDLE, page_size: usize) -> Result<u64, String> {
    let mut addr = 0usize;
    let mut total_pages = 0u64;

    loop {
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { zeroed() };
        let read = unsafe {
            VirtualQueryEx(
                handle,
                addr as *const _,
                &mut info,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if read == 0 {
            break;
        }

        let base = info.BaseAddress as usize;
        let size = info.RegionSize;
        if info.State == MEM_COMMIT && info.Type == MEM_PRIVATE && size > 0 {
            total_pages =
                total_pages.saturating_add(resident_private_pages(handle, base, size, page_size)?);
        }

        let next = base.saturating_add(size);
        if next <= addr {
            break;
        }
        addr = next;
    }

    Ok(total_pages.saturating_mul(page_size as u64))
}

fn resident_private_pages(
    handle: HANDLE,
    base: usize,
    size: usize,
    page_size: usize,
) -> Result<u64, String> {
    const WS_VALID: usize = 1;
    const WS_SHARED: usize = 1 << 15;

    let pages = size.div_ceil(page_size);
    if pages == 0 {
        return Ok(0);
    }

    let mut query: Vec<PSAPI_WORKING_SET_EX_INFORMATION> = (0..pages)
        .map(|i| PSAPI_WORKING_SET_EX_INFORMATION {
            VirtualAddress: (base + i * page_size) as *mut _,
            VirtualAttributes: Default::default(),
        })
        .collect();
    let bytes = query
        .len()
        .checked_mul(size_of::<PSAPI_WORKING_SET_EX_INFORMATION>())
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| "working set query is too large".to_string())?;

    let ok = unsafe { K32QueryWorkingSetEx(handle, query.as_mut_ptr().cast(), bytes) };
    if ok == 0 {
        return Ok(0);
    }

    Ok(query
        .iter()
        .filter(|entry| {
            let flags = unsafe { entry.VirtualAttributes.Flags };
            flags & WS_VALID != 0 && flags & WS_SHARED == 0
        })
        .count() as u64)
}

fn page_size() -> Result<usize, String> {
    let mut info: SYSTEM_INFO = unsafe { zeroed() };
    unsafe {
        GetSystemInfo(&mut info);
    };
    if info.dwPageSize == 0 {
        return Err("could not read system page size".into());
    }
    Ok(info.dwPageSize as usize)
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
