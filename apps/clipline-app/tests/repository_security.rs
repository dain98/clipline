use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf()
}

fn unix_day_for_iso_date(value: &str) -> i64 {
    let mut parts = value.split('-');
    let mut year: i64 = parts
        .next()
        .expect("date year")
        .parse()
        .expect("numeric year");
    let month: i64 = parts
        .next()
        .expect("date month")
        .parse()
        .expect("numeric month");
    let day: i64 = parts
        .next()
        .expect("date day")
        .parse()
        .expect("numeric day");
    assert!(parts.next().is_none(), "date must be YYYY-MM-DD: {value}");
    assert!((1..=12).contains(&month), "invalid date month: {value}");
    assert!((1..=31).contains(&day), "invalid date day: {value}");

    year -= i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn assert_not_past(value: &str, subject: &str) {
    let today = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_secs() as i64
        / 86_400;
    assert!(
        today <= unix_day_for_iso_date(value),
        "{subject} expired on {value}; review and update its policy"
    );
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

#[test]
fn dependency_exceptions_and_fixed_runtime_are_owned_and_current() {
    let root = workspace_root();
    let lock = fs::read_to_string(root.join("Cargo.lock")).expect("read Cargo.lock");
    assert!(
        !lock.contains("name = \"audiopus\"") && !lock.contains("name = \"audiopus_sys\""),
        "the unmaintained audiopus binding must not be selected"
    );
    let opus_packages: Vec<_> = lock
        .split("[[package]]")
        .filter(|package| package.contains("name = \"shiguredo_opus\""))
        .collect();
    assert_eq!(opus_packages.len(), 1, "select one maintained Opus binding");
    assert!(
        opus_packages[0].contains("version = \"2026.1.0\"")
            && !opus_packages[0].contains("source = "),
        "use Clipline's reviewed shiguredo_opus 2026.1.0 controlled fork"
    );

    let mut reqwest_lines: Vec<_> = lock
        .split("[[package]]")
        .filter(|package| package.contains("name = \"reqwest\""))
        .map(|package| {
            let version = package
                .lines()
                .find_map(|line| line.trim().strip_prefix("version = \"")?.strip_suffix('"'))
                .expect("reqwest version");
            version.rsplit_once('.').expect("reqwest patch version").0
        })
        .collect();
    reqwest_lines.sort_unstable();
    assert_eq!(
        reqwest_lines,
        ["0.12", "0.13"],
        "only the two reviewed reqwest release lines may be selected"
    );

    let policy: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join("docs/dependency-policy.json"))
            .expect("read dependency policy"),
    )
    .expect("valid dependency policy JSON");
    let exception = policy["duplicate_major_exceptions"]
        .as_array()
        .expect("duplicate-major exception array")
        .iter()
        .find(|exception| exception["package"] == "reqwest")
        .expect("reqwest duplicate-major exception");
    assert_eq!(
        exception["allowed_versions"],
        serde_json::json!(["0.12", "0.13"])
    );
    for field in ["owner", "rationale", "review_by", "remove_when"] {
        assert!(
            exception[field]
                .as_str()
                .is_some_and(|value| !value.is_empty()),
            "reqwest exception requires {field}"
        );
    }
    assert_not_past(
        exception["review_by"].as_str().expect("review date"),
        "reqwest duplicate-major exception",
    );
    let opus_fork = policy["controlled_forks"]
        .as_array()
        .expect("controlled-fork array")
        .iter()
        .find(|fork| fork["package"] == "shiguredo_opus")
        .expect("controlled Opus fork policy");
    for field in ["owner", "rationale", "review_by", "remove_when", "upstream"] {
        assert!(
            opus_fork[field]
                .as_str()
                .is_some_and(|value| !value.is_empty()),
            "controlled Opus fork requires {field}"
        );
    }
    assert_not_past(
        opus_fork["review_by"].as_str().expect("fork review date"),
        "controlled Opus fork",
    );
    let fork_build = fs::read_to_string(root.join("third-party/shiguredo_opus/build.rs"))
        .expect("read controlled Opus build script");
    for contract in [
        "windows_x86_64",
        "ubuntu-22.04_x86_64",
        "ubuntu-24.04_x86_64",
        "228e55adda46e79b7d5be1950283aa2f79f3de8b19081cb1a6ed74fa71f5f602",
        "opus.lib",
        "no reviewed prebuilt Opus hash",
    ] {
        assert!(
            fork_build.contains(contract),
            "Opus fork must retain {contract}"
        );
    }
    assert!(root.join("third-party/shiguredo_opus/LICENSE").is_file());
    assert!(root
        .join("third-party/shiguredo_opus/CLIPLINE-PATCHES.md")
        .is_file());

    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join("apps/clipline-app/webview2-fixed-runtime.json"))
            .expect("read WebView2 runtime manifest"),
    )
    .expect("valid WebView2 runtime manifest JSON");
    let version = manifest["version"].as_str().expect("runtime version");
    let architecture = manifest["architecture"]
        .as_str()
        .expect("runtime architecture");
    assert_eq!(architecture, "x64");
    assert_not_past(
        manifest["review_due_on"]
            .as_str()
            .expect("runtime review due date"),
        "WebView2 Fixed Version runtime review",
    );
    assert_eq!(manifest["max_review_age_days"], 30);
    assert!(manifest["source_url"]
        .as_str()
        .is_some_and(|url| url.starts_with("https://developer.microsoft.com/")));

    let config = fs::read_to_string(root.join("apps/clipline-app/tauri.standalone.conf.json"))
        .expect("read standalone config");
    let expected_folder =
        format!("Microsoft.WebView2.FixedVersionRuntime.{version}.{architecture}");
    assert_eq!(config.matches(&expected_folder).count(), 2);

    let verifier = fs::read_to_string(root.join("scripts/verify-webview2-runtime.ps1"))
        .expect("read WebView2 runtime verifier");
    for contract in [
        "review_due_on",
        "tauri.standalone.conf.json",
        "Test-Path",
        "RequirePayload",
        "msedgewebview2.exe",
    ] {
        assert!(
            verifier.contains(contract),
            "runtime verifier must enforce {contract}"
        );
    }
}

#[test]
fn ffmpeg_release_staging_is_pinned_allowlisted_and_attributed() {
    let root = workspace_root();
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join("apps/clipline-app/ffmpeg-runtime.json"))
            .expect("read FFmpeg runtime manifest"),
    )
    .expect("valid FFmpeg runtime manifest JSON");

    let release_tag = manifest["release_tag"].as_str().expect("release tag");
    let archive_name = manifest["archive_name"].as_str().expect("archive name");
    let archive_url = manifest["archive_url"].as_str().expect("archive URL");
    let archive_sha = manifest["archive_sha256"]
        .as_str()
        .expect("archive SHA-256");
    assert!(release_tag.starts_with("autobuild-20") && !release_tag.contains("latest"));
    assert!(archive_name.ends_with("win64-lgpl-shared-8.1.zip"));
    assert!(archive_url.starts_with("https://github.com/BtbN/FFmpeg-Builds/releases/download/"));
    assert!(archive_url.contains(release_tag) && archive_url.ends_with(archive_name));
    assert!(!archive_url.contains("/latest/"));
    assert!(
        archive_sha.len() == 64 && archive_sha.chars().all(|ch| ch.is_ascii_hexdigit()),
        "FFmpeg archive requires an exact SHA-256"
    );
    assert!(manifest["version_line"]
        .as_str()
        .is_some_and(|line| line.starts_with("ffmpeg version n8.1.")));
    let forbidden_configuration = manifest["forbidden_configuration"]
        .as_array()
        .expect("forbidden FFmpeg configuration");
    for forbidden in [
        "--enable-gpl",
        "--enable-nonfree",
        "--enable-libx264",
        "--enable-libx265",
    ] {
        assert!(
            forbidden_configuration
                .iter()
                .any(|value| value.as_str() == Some(forbidden)),
            "FFmpeg manifest must reject {forbidden}"
        );
    }

    let files = manifest["allowed_files"]
        .as_array()
        .expect("FFmpeg file allowlist");
    let staged_names: Vec<_> = files
        .iter()
        .map(|file| file["staged_name"].as_str().expect("staged file name"))
        .collect();
    assert_eq!(
        staged_names,
        [
            "LICENSE.txt",
            "ffmpeg.exe",
            "avcodec-62.dll",
            "avdevice-62.dll",
            "avfilter-11.dll",
            "avformat-62.dll",
            "avutil-60.dll",
            "swresample-6.dll",
            "swscale-9.dll",
        ]
    );
    let mut unique_names = staged_names.clone();
    unique_names.sort_unstable();
    unique_names.dedup();
    assert_eq!(unique_names.len(), staged_names.len());
    for file in files {
        let archive_path = file["archive_path"].as_str().expect("archive path");
        let sha = file["sha256"].as_str().expect("file SHA-256");
        assert!(
            !archive_path.starts_with('/')
                && !archive_path.starts_with('\\')
                && !archive_path.contains("..")
        );
        assert!(file["size"].as_u64().is_some_and(|size| size > 0));
        assert!(sha.len() == 64 && sha.chars().all(|ch| ch.is_ascii_hexdigit()));
    }
    assert!(!staged_names.contains(&"ffplay.exe"));
    assert!(!staged_names.contains(&"ffprobe.exe"));

    let script = fs::read_to_string(root.join("scripts/stage-ffmpeg-resource.ps1"))
        .expect("read FFmpeg staging script");
    for contract in [
        "Get-FileHash",
        "OpenRead",
        "allowed_files",
        "PROVENANCE.json",
        "version_line",
        "forbidden_configuration",
        "Move-Item",
    ] {
        assert!(
            script.contains(contract),
            "FFmpeg staging must enforce {contract}"
        );
    }
    assert!(!script.contains("$SourceDir"));

    let tauri = fs::read_to_string(root.join("apps/clipline-app/tauri.conf.json"))
        .expect("read Tauri config");
    assert_eq!(tauri.matches("\"ffmpeg/\"").count(), 1);
    let readme = fs::read_to_string(root.join("apps/clipline-app/ffmpeg/README.md"))
        .expect("read bundled FFmpeg notice");
    assert!(readme.contains("LGPL") && readme.contains("replace"));
    let notices =
        fs::read_to_string(root.join("THIRD-PARTY-NOTICES.md")).expect("read third-party notices");
    for provenance in [release_tag, "ce3c09c101", "PROVENANCE.json", "LGPL v3"] {
        assert!(
            notices.contains(provenance),
            "FFmpeg notice must retain {provenance}"
        );
    }
}
