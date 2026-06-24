use std::collections::VecDeque;
use std::process::Command;

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct MemoryStatus {
    pub private_working_set_bytes: u64,
}

pub fn current_process_tree_memory() -> Result<MemoryStatus, String> {
    let root = std::process::id();
    let mut total = 0u64;
    let mut queue = VecDeque::from([root]);
    while let Some(pid) = queue.pop_front() {
        total = total.saturating_add(rss_bytes_for_pid(pid)?);
        for child in child_pids(pid) {
            queue.push_back(child);
        }
    }
    Ok(MemoryStatus {
        private_working_set_bytes: total,
    })
}

fn rss_bytes_for_pid(pid: u32) -> Result<u64, String> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .map_err(|e| format!("read process memory: {e}"))?;
    if !output.status.success() {
        return Ok(0);
    }
    parse_ps_rss_kib(&String::from_utf8_lossy(&output.stdout))
}

fn child_pids(pid: u32) -> Vec<u32> {
    let Ok(output) = Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_pgrep_children(&String::from_utf8_lossy(&output.stdout))
}

fn parse_ps_rss_kib(output: &str) -> Result<u64, String> {
    let kib = output
        .split_whitespace()
        .next()
        .ok_or_else(|| "process memory output was empty".to_string())?
        .parse::<u64>()
        .map_err(|e| format!("parse process memory: {e}"))?;
    Ok(kib.saturating_mul(1024))
}

fn parse_pgrep_children(output: &str) -> Vec<u32> {
    output
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ps_rss_kib_reads_first_number_as_bytes() {
        assert_eq!(parse_ps_rss_kib("  2048\n").unwrap(), 2 * 1024 * 1024);
    }

    #[test]
    fn parse_ps_rss_kib_rejects_empty_output() {
        assert!(parse_ps_rss_kib("   \n").is_err());
    }

    #[test]
    fn parse_pgrep_children_skips_invalid_lines() {
        assert_eq!(parse_pgrep_children("12\nbad\n34\n"), vec![12, 34]);
    }
}
