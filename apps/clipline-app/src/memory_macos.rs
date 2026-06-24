#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct MemoryStatus {
    pub private_working_set_bytes: u64,
}

#[allow(dead_code)]
// Kept for API parity with Windows while the platform facade owns the Milestone 1 stub path.
pub fn current_process_tree_memory() -> Result<MemoryStatus, String> {
    Err("macOS memory status is not implemented in Milestone 1".into())
}
