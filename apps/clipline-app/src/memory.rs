use std::collections::{HashMap, HashSet};
use std::mem::{size_of, zeroed};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::ProcessStatus::{
    K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct MemoryStatus {
    pub private_working_set_bytes: u64,
}

const MEMORY_SAMPLE_CACHE_TTL: Duration = Duration::from_secs(1);
const MEMORY_QUERY_ACCESS: u32 = PROCESS_QUERY_LIMITED_INFORMATION;

struct CachedMemorySample {
    completed_at: Instant,
    result: Result<MemoryStatus, String>,
}

pub struct MemorySampler {
    cache_ttl: Duration,
    state: tokio::sync::Mutex<Option<CachedMemorySample>>,
}

impl Default for MemorySampler {
    fn default() -> Self {
        Self::with_cache_ttl(MEMORY_SAMPLE_CACHE_TTL)
    }
}

impl MemorySampler {
    fn with_cache_ttl(cache_ttl: Duration) -> Self {
        Self {
            cache_ttl,
            state: tokio::sync::Mutex::new(None),
        }
    }

    pub async fn sample(&self) -> Result<MemoryStatus, String> {
        self.sample_with(current_process_tree_memory).await
    }

    async fn sample_with(
        &self,
        measure: impl FnOnce() -> Result<MemoryStatus, String> + Send + 'static,
    ) -> Result<MemoryStatus, String> {
        let mut state = self.state.lock().await;
        if let Some(cached) = state.as_ref() {
            if cached.completed_at.elapsed() < self.cache_ttl {
                return cached.result.clone();
            }
        }
        let result = tauri::async_runtime::spawn_blocking(measure)
            .await
            .map_err(|error| format!("memory sampler task failed: {error}"))?;
        *state = Some(CachedMemorySample {
            completed_at: Instant::now(),
            result: result.clone(),
        });
        result
    }
}

pub fn current_process_tree_memory() -> Result<MemoryStatus, String> {
    let current_pid = unsafe { GetCurrentProcessId() };
    let mut total = query_process_private_working_set(unsafe { GetCurrentProcess() })?;

    for pid in child_process_ids(current_pid)? {
        if let Ok(bytes) = query_process_private_working_set_for_pid(pid) {
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

fn query_process_private_working_set_for_pid(pid: u32) -> Result<u64, String> {
    let handle = unsafe { OpenProcess(MEMORY_QUERY_ACCESS, 0, pid) };
    if handle.is_null() {
        return Err(format!(
            "open process {pid} for memory counters: {}",
            std::io::Error::last_os_error()
        ));
    }
    let result = query_process_private_working_set(handle);
    unsafe {
        CloseHandle(handle);
    }
    result
}

fn query_process_private_working_set(handle: HANDLE) -> Result<u64, String> {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            handle,
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX2).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    };
    if ok == 0 {
        return Err(format!(
            "query process memory counters: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(counters.PrivateWorkingSetSize as u64)
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

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
    fn memory_sampling_requires_only_limited_query_rights() {
        assert_eq!(MEMORY_QUERY_ACCESS, PROCESS_QUERY_LIMITED_INFORMATION);
    }

    #[test]
    fn current_process_private_working_set_is_available_without_vm_read() {
        let bytes = query_process_private_working_set_for_pid(std::process::id())
            .expect("the current process should expose extended memory counters");

        assert!(bytes > 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn memory_sampler_coalesces_concurrent_measurements() {
        let sampler = Arc::new(MemorySampler::with_cache_ttl(Duration::from_secs(1)));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let sampler = Arc::clone(&sampler);
            let calls = Arc::clone(&calls);
            tasks.push(tokio::spawn(async move {
                sampler
                    .sample_with(move || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(25));
                        Ok(MemoryStatus {
                            private_working_set_bytes: 42,
                        })
                    })
                    .await
            }));
        }

        for task in tasks {
            assert_eq!(task.await.unwrap().unwrap().private_working_set_bytes, 42);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn memory_sampler_caches_failures_but_retries_after_expiry() {
        let sampler = MemorySampler::with_cache_ttl(Duration::from_millis(1));
        let calls = Arc::new(AtomicUsize::new(0));

        let first_calls = Arc::clone(&calls);
        assert_eq!(
            sampler
                .sample_with(move || {
                    first_calls.fetch_add(1, Ordering::SeqCst);
                    Err("sample failed".to_string())
                })
                .await,
            Err("sample failed".to_string())
        );
        let cached_calls = Arc::clone(&calls);
        assert_eq!(
            sampler
                .sample_with(move || {
                    cached_calls.fetch_add(1, Ordering::SeqCst);
                    Ok(MemoryStatus {
                        private_working_set_bytes: 7,
                    })
                })
                .await,
            Err("sample failed".to_string())
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
        let retry_calls = Arc::clone(&calls);
        assert_eq!(
            sampler
                .sample_with(move || {
                    retry_calls.fetch_add(1, Ordering::SeqCst);
                    Ok(MemoryStatus {
                        private_working_set_bytes: 7,
                    })
                })
                .await
                .unwrap()
                .private_working_set_bytes,
            7
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
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
