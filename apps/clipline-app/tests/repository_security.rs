use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn dependency_and_ci_supply_chain_is_reviewable_and_audited() {
    let root = workspace_root();
    let workflows = root.join(".github/workflows");
    let mut saw_rustsec = false;

    for entry in fs::read_dir(&workflows).expect("read workflows") {
        let path = entry.expect("workflow entry").path();
        if !matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("yml" | "yaml")
        ) {
            continue;
        }
        let workflow = fs::read_to_string(&path).expect("read workflow");
        saw_rustsec |= workflow.contains("rustsec/audit-check@");
        for line in workflow.lines() {
            let Some(spec) = line.trim().strip_prefix("- uses:") else {
                continue;
            };
            let spec = spec.trim();
            if spec.starts_with("./") {
                continue;
            }
            let (action, revision_and_comment) = spec.split_once('@').unwrap_or_else(|| {
                panic!("remote action lacks revision in {}: {line}", path.display())
            });
            let revision = revision_and_comment
                .split_whitespace()
                .next()
                .expect("action revision");
            assert!(
                revision.len() == 40 && revision.chars().all(|ch| ch.is_ascii_hexdigit()),
                "{action} must use a full commit SHA in {}: {line}",
                path.display()
            );
            assert!(
                line.contains('#'),
                "pinned action needs a readable version comment in {}: {line}",
                path.display()
            );
        }
    }
    assert!(saw_rustsec, "a pinned RustSec audit workflow is required");

    let audit_policy =
        fs::read_to_string(root.join(".cargo/audit.toml")).expect("read audit policy");
    assert!(audit_policy.contains("ignore = []"));
    for requirement in ["owner", "rationale", "expiry", "removal"] {
        assert!(
            audit_policy.to_ascii_lowercase().contains(requirement),
            "audit ignore policy must document {requirement}"
        );
    }

    let dependabot =
        fs::read_to_string(root.join(".github/dependabot.yml")).expect("read Dependabot config");
    assert!(dependabot.contains("package-ecosystem: cargo"));
    assert!(dependabot.contains("package-ecosystem: github-actions"));

    let lock = fs::read_to_string(root.join("Cargo.lock")).expect("read Cargo.lock");
    for (crate_name, minimum, advisories) in [
        ("anyhow", &[1, 0, 103][..], "RUSTSEC-2026-0190"),
        ("quick-xml", &[0, 41, 0][..], "RUSTSEC-2026-0194/0195"),
        ("quinn-proto", &[0, 11, 15][..], "RUSTSEC-2026-0185"),
    ] {
        let packages: Vec<_> = lock
            .split("[[package]]")
            .filter(|package| {
                package
                    .lines()
                    .any(|line| line.trim() == format!("name = \"{crate_name}\""))
            })
            .collect();
        assert!(!packages.is_empty(), "missing locked {crate_name} package");
        for package in packages {
            let version = package
                .lines()
                .find_map(|line| line.trim().strip_prefix("version = \"")?.strip_suffix('"'))
                .expect("package version");
            let parts: Vec<u64> = version
                .split('.')
                .map(|part| part.parse().expect("numeric package version"))
                .collect();
            assert!(
                parts.as_slice() >= minimum,
                "{crate_name} {version} is affected by {advisories}"
            );
        }
    }
}
