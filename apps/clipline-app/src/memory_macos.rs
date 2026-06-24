#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct MemoryStatus {
    pub private_working_set_bytes: u64,
}

pub fn current_process_tree_memory() -> Result<MemoryStatus, String> {
    Ok(MemoryStatus {
        private_working_set_bytes: 0,
    })
}
