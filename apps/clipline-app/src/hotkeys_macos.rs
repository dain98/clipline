pub fn install_save_hook<F>(_hotkey: &str, _on_trigger: F) -> Result<(), String>
where
    F: Fn() + Send + Sync + 'static,
{
    Err("macOS focused-game hotkey fallback is not implemented in Milestone 1".into())
}

pub fn set_save_hotkey(_hotkey: &str) -> Result<(), String> {
    Ok(())
}
