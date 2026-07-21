//! Small, deterministic transaction helpers for Credential Manager updates.
//!
//! Windows credentials and `settings.json` cannot share a native transaction.
//! These helpers make the compensating step explicit and testable.

pub(crate) fn write_then_persist<T>(
    target: &str,
    username: &str,
    new_secret: &str,
    previous_secret: Option<&str>,
    mut write: impl FnMut(&str, &str, &str) -> Result<(), String>,
    mut delete: impl FnMut(&str) -> Result<(), String>,
    persist: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    write(target, username, new_secret)?;
    match persist() {
        Ok(value) => Ok(value),
        Err(primary) => {
            let rollback = match previous_secret {
                Some(previous) => write(target, username, previous),
                None => delete(target),
            };
            match rollback {
                Ok(()) => Err(primary),
                Err(rollback) => Err(format!(
                    "{primary}; credential rollback failed for {target:?}: {rollback}"
                )),
            }
        }
    }
}

pub(crate) struct CleanupReport {
    pub deleted: Vec<String>,
    pub failures: Vec<String>,
}

pub(crate) fn cleanup_targets(
    targets: Vec<String>,
    mut delete: impl FnMut(&str) -> Result<(), String>,
) -> CleanupReport {
    let mut report = CleanupReport {
        deleted: Vec::new(),
        failures: Vec::new(),
    };
    for target in targets {
        match delete(&target) {
            Ok(()) => report.deleted.push(target),
            Err(error) => report.failures.push(format!("{target:?}: {error}")),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_credential_is_deleted_when_persistence_fails() {
        let mut writes = Vec::new();
        let mut deletes = Vec::new();

        let error = write_then_persist(
            "target",
            "user",
            "new",
            None,
            |target, user, secret| {
                writes.push((target.to_string(), user.to_string(), secret.to_string()));
                Ok(())
            },
            |target| {
                deletes.push(target.to_string());
                Ok(())
            },
            || Err::<(), _>("settings save failed".into()),
        )
        .unwrap_err();

        assert_eq!(error, "settings save failed");
        assert_eq!(writes, [("target".into(), "user".into(), "new".into())]);
        assert_eq!(deletes, ["target"]);
    }

    #[test]
    fn overwritten_credential_is_restored_when_persistence_fails() {
        let mut secrets = Vec::new();

        let error = write_then_persist(
            "target",
            "user",
            "new",
            Some("old"),
            |_, _, secret| {
                secrets.push(secret.to_string());
                Ok(())
            },
            |_| Ok(()),
            || Err::<(), _>("settings save failed".into()),
        )
        .unwrap_err();

        assert_eq!(error, "settings save failed");
        assert_eq!(secrets, ["new", "old"]);
    }

    #[test]
    fn rollback_failure_is_not_hidden() {
        let error = write_then_persist(
            "target",
            "user",
            "new",
            None,
            |_, _, _| Ok(()),
            |_| Err("credential store unavailable".into()),
            || Err::<(), _>("settings save failed".into()),
        )
        .unwrap_err();

        assert!(error.contains("settings save failed"), "{error}");
        assert!(error.contains("rollback failed"), "{error}");
        assert!(error.contains("credential store unavailable"), "{error}");
    }

    #[test]
    fn cleanup_report_keeps_failed_targets_for_reconciliation() {
        let report = cleanup_targets(vec!["old-1".into(), "old-2".into()], |target| {
            if target == "old-2" {
                Err("store busy".into())
            } else {
                Ok(())
            }
        });

        assert_eq!(report.deleted, ["old-1"]);
        assert_eq!(report.failures.len(), 1);
        assert!(report.failures[0].contains("old-2"));
    }
}
