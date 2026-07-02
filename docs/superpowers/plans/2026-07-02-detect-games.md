# Detect Games Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a manual Detect Games workflow that scans Steam installs plus running visible windows, shows unchecked detected candidates, and appends selected rows into the existing Custom games list.

**Architecture:** Add a focused `game_discovery` module in the Windows app crate for Steam VDF parsing, Steam catalog scanning, running-window candidate creation, merging, filtering, sorting, and custom-game dedupe. Expose it through an async Tauri command that receives the current frontend custom-game draft, then add a small inline Settings > Games panel that converts selected candidates into normal `CustomGameSettings`.

**Tech Stack:** Rust/Tauri 2, existing `clipline_capture::windows::CapturableWindow`, `clipline-test-utils::TestDir`, vanilla HTML/CSS/JS, Rust UI contract tests, workspace Cargo tests and clippy.

---

## File Structure

- Create `apps/clipline-app/src/game_discovery.rs`
  - Owns detected-game candidate structs, Steam VDF parsing, Steam library/app discovery, executable inference, running-window candidates, merge/dedupe/filter/sort logic, and backend tests.
- Modify `apps/clipline-app/src/main.rs`
  - Registers `mod game_discovery;` behind the existing Windows gate.
- Modify `apps/clipline-app/src/app.rs`
  - Imports `DetectedGameCandidate`, adds async `detect_installed_games(existing_custom_games: Vec<CustomGameSettings>)`, and registers it in `tauri::generate_handler!`.
- Modify `apps/clipline-app/ui/app-core.js`
  - Adds `detectedGameCandidates` and `selectedDetectedGameIds` global state.
- Modify `apps/clipline-app/ui/index.html`
  - Adds `Detect Games`, `detected-games-panel`, `detected-games-list`, `add-detected-games`, and `cancel-detected-games` controls beside the existing Custom games UI.
- Modify `apps/clipline-app/ui/settings.js`
  - Adds detect-game render, scan, selection, dedupe, and add-selected logic.
- Modify `apps/clipline-app/ui/main.js`
  - Wires Detect Games/Add game(s)/Cancel events.
- Modify `apps/clipline-app/ui/styles.css`
  - Reuses the existing custom-game/game-window visual language for detected rows and the inline panel.
- Modify `apps/clipline-app/tests/ui_contract.rs`
  - Extends existing Games UI contract tests for markup, command invoke, and event wiring.
- Modify `handoff.md`
  - Adds a short note after implementation because this is a significant Settings > Games workflow.

---

### Task 1: Discovery Model and VDF Parser

**Files:**
- Create: `apps/clipline-app/src/game_discovery.rs`
- Modify: `apps/clipline-app/src/main.rs`
- Test: `apps/clipline-app/src/game_discovery.rs`

- [ ] **Step 1: Write failing parser/model tests**

Create `apps/clipline-app/src/game_discovery.rs` with only the test module below first. Do not add production structs or parser code yet.

```rust
#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn parses_libraryfolders_paths_from_keyvalue_vdf() {
        let input = r#"
            "libraryfolders"
            {
                "0"
                {
                    "path" "C:\\Program Files (x86)\\Steam"
                    "apps"
                    {
                        "570" "12345"
                    }
                }
                "1"
                {
                    "path" "D:\\SteamLibrary"
                }
                "2" "E:\\LegacySteamLibrary"
            }
        "#;

        let parsed = parse_vdf(input).expect("libraryfolders parses");
        assert_eq!(
            library_paths_from_vdf(&parsed),
            vec![
                PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from(r"D:\SteamLibrary"),
                PathBuf::from(r"E:\LegacySteamLibrary"),
            ]
        );
    }

    #[test]
    fn parses_appmanifest_core_fields() {
        let input = r#"
            "AppState"
            {
                "appid" "646570"
                "name" "Slay the Spire"
                "installdir" "SlayTheSpire"
                "StateFlags" "4"
            }
        "#;

        let parsed = parse_vdf(input).expect("appmanifest parses");
        let manifest = steam_app_from_manifest(&parsed).expect("manifest fields");

        assert_eq!(manifest.app_id, 646570);
        assert_eq!(manifest.name, "Slay the Spire");
        assert_eq!(manifest.install_dir_name, "SlayTheSpire");
    }

    #[test]
    fn malformed_vdf_returns_error() {
        let err = parse_vdf(r#""libraryfolders" { "0" { "path" "C:\\Steam""#)
            .expect_err("unclosed object should fail");
        assert!(
            err.contains("unterminated object"),
            "unexpected parse error: {err}"
        );
    }
}
```

Add the module declaration in `apps/clipline-app/src/main.rs` at the Windows module list:

```rust
#[cfg(windows)]
mod game_discovery;
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```powershell
cargo test -p clipline-app game_discovery::tests::parses_libraryfolders_paths_from_keyvalue_vdf game_discovery::tests::parses_appmanifest_core_fields game_discovery::tests::malformed_vdf_returns_error
```

Expected: FAIL because `parse_vdf`, `library_paths_from_vdf`, and `steam_app_from_manifest` are not defined.

- [ ] **Step 3: Implement the model and parser**

At the top of `apps/clipline-app/src/game_discovery.rs`, add the public candidate types and private VDF helpers. Keep this code self-contained and dependency-free.

```rust
use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectedGameSource {
    Steam,
    RunningWindow,
    SteamAndRunningWindow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectedGameCandidate {
    pub id_hint: String,
    pub name: String,
    pub source: DetectedGameSource,
    pub steam_app_id: Option<u32>,
    pub install_dir: Option<String>,
    pub exe_name: String,
    pub process_path: Option<String>,
    pub window_title: String,
    pub icon: Option<String>,
    pub confidence: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SteamAppManifest {
    app_id: u32,
    name: String,
    install_dir_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum VdfEntry {
    Pair { key: String, value: String },
    Object { key: String, children: Vec<VdfEntry> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum VdfToken {
    String(String),
    Open,
    Close,
}
```

Implement tokenization and parsing in the same file. The parser only needs quoted key/value strings plus `{` and `}`:

```rust
fn parse_vdf(input: &str) -> Result<Vec<VdfEntry>, String> {
    let tokens = tokenize_vdf(input)?;
    let mut index = 0;
    parse_vdf_entries(&tokens, &mut index, false)
}

fn tokenize_vdf(input: &str) -> Result<Vec<VdfToken>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            c if c.is_whitespace() => {}
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '{' => tokens.push(VdfToken::Open),
            '}' => tokens.push(VdfToken::Close),
            '"' => {
                let mut value = String::new();
                let mut closed = false;
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(next) = chars.next() {
                                value.push(next);
                            }
                        }
                        '"' => {
                            closed = true;
                            break;
                        }
                        other => value.push(other),
                    }
                }
                if !closed {
                    return Err("unterminated string".into());
                }
                tokens.push(VdfToken::String(value));
            }
            other => return Err(format!("unexpected VDF character {other:?}")),
        }
    }
    Ok(tokens)
}

fn parse_vdf_entries(
    tokens: &[VdfToken],
    index: &mut usize,
    inside_object: bool,
) -> Result<Vec<VdfEntry>, String> {
    let mut entries = Vec::new();
    while *index < tokens.len() {
        match &tokens[*index] {
            VdfToken::Close if inside_object => {
                *index += 1;
                return Ok(entries);
            }
            VdfToken::Close => return Err("unexpected object close".into()),
            VdfToken::Open => return Err("unexpected object open".into()),
            VdfToken::String(key) => {
                let key = key.clone();
                *index += 1;
                match tokens.get(*index) {
                    Some(VdfToken::String(value)) => {
                        entries.push(VdfEntry::Pair {
                            key,
                            value: value.clone(),
                        });
                        *index += 1;
                    }
                    Some(VdfToken::Open) => {
                        *index += 1;
                        let children = parse_vdf_entries(tokens, index, true)?;
                        entries.push(VdfEntry::Object { key, children });
                    }
                    Some(VdfToken::Close) | None => {
                        return Err(format!("missing value for key {key:?}"));
                    }
                }
            }
        }
    }
    if inside_object {
        Err("unterminated object".into())
    } else {
        Ok(entries)
    }
}
```

Add helpers used by the tests:

```rust
fn library_paths_from_vdf(entries: &[VdfEntry]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let Some(root) = find_object(entries, "libraryfolders") else {
        return paths;
    };
    for entry in root {
        match entry {
            VdfEntry::Object { children, .. } => {
                if let Some(path) = find_pair(children, "path") {
                    paths.push(PathBuf::from(path));
                }
            }
            VdfEntry::Pair { value, .. } => paths.push(PathBuf::from(value)),
        }
    }
    paths
}

fn steam_app_from_manifest(entries: &[VdfEntry]) -> Option<SteamAppManifest> {
    let app_state = find_object(entries, "AppState")?;
    Some(SteamAppManifest {
        app_id: find_pair(app_state, "appid")?.parse().ok()?,
        name: find_pair(app_state, "name")?.to_string(),
        install_dir_name: find_pair(app_state, "installdir")?.to_string(),
    })
}

fn find_object<'a>(entries: &'a [VdfEntry], key: &str) -> Option<&'a [VdfEntry]> {
    entries.iter().find_map(|entry| match entry {
        VdfEntry::Object { key: found, children } if found.eq_ignore_ascii_case(key) => {
            Some(children.as_slice())
        }
        _ => None,
    })
}

fn find_pair<'a>(entries: &'a [VdfEntry], key: &str) -> Option<&'a str> {
    entries.iter().find_map(|entry| match entry {
        VdfEntry::Pair { key: found, value } if found.eq_ignore_ascii_case(key) => {
            Some(value.as_str())
        }
        _ => None,
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run:

```powershell
cargo test -p clipline-app game_discovery::tests::parses_libraryfolders_paths_from_keyvalue_vdf game_discovery::tests::parses_appmanifest_core_fields game_discovery::tests::malformed_vdf_returns_error
```

Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add apps/clipline-app/src/main.rs apps/clipline-app/src/game_discovery.rs
git commit -m "feat(games): add Steam discovery parser"
```

---

### Task 2: Steam Catalog Scan and Executable Inference

**Files:**
- Modify: `apps/clipline-app/src/game_discovery.rs`
- Test: `apps/clipline-app/src/game_discovery.rs`

- [ ] **Step 1: Write failing Steam catalog tests**

Append these tests inside the existing `#[cfg(test)] mod tests` in `game_discovery.rs`:

```rust
use clipline_test_utils::TestDir;

fn vdf_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "\\\\")
}

#[test]
fn steam_catalog_reads_manifests_and_infers_best_executable() {
    let dir = TestDir::new("clipline-game-discovery", "steam-catalog");
    let steam_root = dir.path().join("Steam");
    let library = dir.path().join("Library");
    std::fs::create_dir_all(steam_root.join("steamapps")).unwrap();
    std::fs::create_dir_all(library.join("steamapps/common/SlayTheSpire")).unwrap();
    std::fs::write(
        steam_root.join("steamapps/libraryfolders.vdf"),
        format!(
            r#""libraryfolders" {{ "0" {{ "path" "{}" }} "1" {{ "path" "{}" }} }}"#,
            vdf_path(&steam_root),
            vdf_path(&library)
        ),
    )
    .unwrap();
    std::fs::write(
        library.join("steamapps/appmanifest_646570.acf"),
        r#""AppState" { "appid" "646570" "name" "Slay the Spire" "installdir" "SlayTheSpire" }"#,
    )
    .unwrap();
    std::fs::write(library.join("steamapps/common/SlayTheSpire/UnityCrashHandler64.exe"), b"")
        .unwrap();
    std::fs::write(library.join("steamapps/common/SlayTheSpire/SlayTheSpire.exe"), b"")
        .unwrap();

    let apps = steam_apps_from_roots(&[steam_root]).expect("steam scan");
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].app_id, 646570);
    assert_eq!(apps[0].name, "Slay the Spire");
    assert_eq!(
        apps[0].install_dir,
        library.join("steamapps/common/SlayTheSpire")
    );
    assert_eq!(apps[0].exe_name.as_deref(), Some("SlayTheSpire.exe"));
}

#[test]
fn steam_catalog_skips_malformed_manifest_and_continues() {
    let dir = TestDir::new("clipline-game-discovery", "steam-malformed");
    let steam_root = dir.path().join("Steam");
    let app_dir = steam_root.join("steamapps/common/Factorio");
    std::fs::create_dir_all(&app_dir).unwrap();
    std::fs::write(
        steam_root.join("steamapps/libraryfolders.vdf"),
        format!(r#""libraryfolders" {{ "0" {{ "path" "{}" }} }}"#, vdf_path(&steam_root)),
    )
    .unwrap();
    std::fs::write(
        steam_root.join("steamapps/appmanifest_bad.acf"),
        r#""AppState" { "appid" "#,
    )
    .unwrap();
    std::fs::write(
        steam_root.join("steamapps/appmanifest_427520.acf"),
        r#""AppState" { "appid" "427520" "name" "Factorio" "installdir" "Factorio" }"#,
    )
    .unwrap();
    std::fs::write(app_dir.join("factorio.exe"), b"").unwrap();

    let apps = steam_apps_from_roots(&[steam_root]).expect("steam scan");
    assert_eq!(apps.iter().map(|app| app.name.as_str()).collect::<Vec<_>>(), vec!["Factorio"]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```powershell
cargo test -p clipline-app game_discovery::tests::steam_catalog_reads_manifests_and_infers_best_executable game_discovery::tests::steam_catalog_skips_malformed_manifest_and_continues
```

Expected: FAIL because `steam_apps_from_roots` and the richer Steam app model do not exist.

- [ ] **Step 3: Implement Steam catalog scanning**

Add this private model near `SteamAppManifest`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
struct SteamApp {
    app_id: u32,
    name: String,
    install_dir: PathBuf,
    exe_name: Option<String>,
    process_path: Option<PathBuf>,
}
```

Add scan functions:

```rust
fn steam_apps_from_roots(steam_roots: &[PathBuf]) -> Result<Vec<SteamApp>, String> {
    let mut libraries = Vec::new();
    for root in steam_roots {
        add_unique_path(&mut libraries, root.clone());
        for library in steam_libraries_from_root(root)? {
            add_unique_path(&mut libraries, library);
        }
    }

    let mut apps = Vec::new();
    for library in libraries {
        let steamapps = library.join("steamapps");
        let Ok(entries) = std::fs::read_dir(&steamapps) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
                continue;
            }
            let Ok(contents) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(vdf) = parse_vdf(&contents) else {
                continue;
            };
            let Some(manifest) = steam_app_from_manifest(&vdf) else {
                continue;
            };
            let install_dir = steamapps.join("common").join(&manifest.install_dir_name);
            let process_path = infer_executable_path(&install_dir, &manifest.name);
            apps.push(SteamApp {
                app_id: manifest.app_id,
                name: manifest.name,
                exe_name: process_path
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .and_then(|name| name.to_str())
                    .map(str::to_string),
                process_path,
                install_dir,
            });
        }
    }
    apps.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
    Ok(apps)
}

fn steam_libraries_from_root(root: &Path) -> Result<Vec<PathBuf>, String> {
    let path = root.join("steamapps/libraryfolders.vdf");
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(_) => return Ok(Vec::new()),
    };
    let parsed = parse_vdf(&contents)?;
    Ok(library_paths_from_vdf(&parsed))
}

fn add_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    let key = path_key(&path);
    if !paths.iter().any(|existing| path_key(existing) == key) {
        paths.push(path);
    }
}
```

Add bounded executable inference and helper scoring:

```rust
fn infer_executable_path(install_dir: &Path, game_name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    collect_exe_candidates(install_dir, 0, &mut candidates);
    candidates
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| !is_helper_exe_name(name))
        })
        .max_by_key(|path| executable_score(path, install_dir, game_name))
}

fn collect_exe_candidates(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 3 || out.len() >= 200 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_exe_candidates(&path, depth + 1, out);
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        {
            out.push(path);
        }
    }
}

fn executable_score(path: &Path, install_dir: &Path, game_name: &str) -> u16 {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(normalized_name_key)
        .unwrap_or_default();
    let game = normalized_name_key(game_name);
    let folder = install_dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(normalized_name_key)
        .unwrap_or_default();
    let mut score = 10;
    if !game.is_empty() && stem == game {
        score += 80;
    }
    if !folder.is_empty() && stem == folder {
        score += 70;
    }
    if !game.is_empty() && (stem.contains(&game) || game.contains(&stem)) {
        score += 35;
    }
    if path.parent() == Some(install_dir) {
        score += 20;
    }
    score
}

fn is_helper_exe_name(name: &str) -> bool {
    let key = name.to_ascii_lowercase();
    [
        "crash", "crashhandler", "unitycrashhandler", "redist", "redistributable",
        "vcredist", "dxsetup", "setup", "install", "unins", "uninstall", "updater",
        "launcherhelper", "helper",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

fn normalized_name_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_ascii_lowercase()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run:

```powershell
cargo test -p clipline-app game_discovery::tests::steam_catalog_reads_manifests_and_infers_best_executable game_discovery::tests::steam_catalog_skips_malformed_manifest_and_continues
```

Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add apps/clipline-app/src/game_discovery.rs
git commit -m "feat(games): scan Steam library manifests"
```

---

### Task 3: Candidate Merge, Filter, Dedupe, and Sort

**Files:**
- Modify: `apps/clipline-app/src/game_discovery.rs`
- Test: `apps/clipline-app/src/game_discovery.rs`

- [ ] **Step 1: Write failing candidate behavior tests**

Append these tests:

```rust
use clipline_capture::windows::CapturableWindow;
use crate::settings::CustomGameSettings;

fn window(title: &str, exe_name: &str, exe_path: Option<&str>) -> CapturableWindow {
    CapturableWindow {
        handle: 42,
        title: title.into(),
        process_id: 101,
        exe_name: exe_name.into(),
        exe_path: exe_path.map(str::to_string),
    }
}

#[test]
fn running_window_under_steam_install_upgrades_steam_candidate() {
    let steam = SteamApp {
        app_id: 646570,
        name: "Slay the Spire".into(),
        install_dir: PathBuf::from(r"C:\Steam\steamapps\common\SlayTheSpire"),
        exe_name: Some("SlayTheSpire.exe".into()),
        process_path: Some(PathBuf::from(r"C:\Steam\steamapps\common\SlayTheSpire\SlayTheSpire.exe")),
    };
    let candidates = candidates_from_sources(
        vec![steam],
        vec![window(
            "Slay the Spire",
            "SlayTheSpire.exe",
            Some(r"C:\Steam\steamapps\common\SlayTheSpire\SlayTheSpire.exe"),
        )],
        &[],
        |_| None,
    );

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].source, DetectedGameSource::SteamAndRunningWindow);
    assert_eq!(candidates[0].window_title, "Slay the Spire");
    assert_eq!(candidates[0].confidence, 100);
}

#[test]
fn keeps_running_non_steam_window_as_candidate() {
    let candidates = candidates_from_sources(
        Vec::new(),
        vec![window("FINAL FANTASY XIV", "ffxiv_dx11.exe", Some(r"D:\Games\FFXIV\ffxiv_dx11.exe"))],
        &[],
        |_| None,
    );

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].name, "ffxiv_dx11");
    assert_eq!(candidates[0].source, DetectedGameSource::RunningWindow);
    assert_eq!(candidates[0].exe_name, "ffxiv_dx11.exe");
}

#[test]
fn filters_launcher_browser_and_helper_windows() {
    let candidates = candidates_from_sources(
        Vec::new(),
        vec![
            window("Steam", "steam.exe", Some(r"C:\Program Files (x86)\Steam\steam.exe")),
            window("Docs", "chrome.exe", Some(r"C:\Program Files\Google\Chrome\Application\chrome.exe")),
            window("Game Crash Handler", "UnityCrashHandler64.exe", Some(r"D:\Game\UnityCrashHandler64.exe")),
            window("33 Immortals", "33Immortals.exe", Some(r"D:\Game\33Immortals.exe")),
        ],
        &[],
        |_| None,
    );

    assert_eq!(candidates.iter().map(|c| c.exe_name.as_str()).collect::<Vec<_>>(), vec!["33Immortals.exe"]);
}

#[test]
fn dedupes_against_existing_custom_games() {
    let existing = CustomGameSettings {
        id: "custom-factorio".into(),
        name: "factorio".into(),
        enabled: true,
        exe_name: "factorio.exe".into(),
        process_path: Some(r"D:\Steam\steamapps\common\Factorio\bin\x64\factorio.exe".into()),
        window_title: "Factorio".into(),
        recording_mode: crate::settings::GameRecordingMode::ReplaysOnly,
        icon: None,
    };
    let candidates = candidates_from_sources(
        Vec::new(),
        vec![window(
            "Factorio: Space Age",
            "factorio.exe",
            Some(r"D:\Steam\steamapps\common\Factorio\bin\x64\factorio.exe"),
        )],
        &[existing],
        |_| None,
    );

    assert!(candidates.is_empty());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```powershell
cargo test -p clipline-app game_discovery::tests::running_window_under_steam_install_upgrades_steam_candidate game_discovery::tests::keeps_running_non_steam_window_as_candidate game_discovery::tests::filters_launcher_browser_and_helper_windows game_discovery::tests::dedupes_against_existing_custom_games
```

Expected: FAIL because `candidates_from_sources` is not implemented.

- [ ] **Step 3: Implement candidate creation and dedupe**

Add imports at the top of `game_discovery.rs`:

```rust
use clipline_capture::windows::{enumerate_capturable_windows, CapturableWindow};

use crate::game_icon;
use crate::settings::CustomGameSettings;
```

Add the public scan entry used by the Tauri command:

```rust
pub fn detect_installed_games(existing_custom_games: &[CustomGameSettings]) -> Vec<DetectedGameCandidate> {
    let steam_apps = steam_install_roots()
        .and_then(|roots| steam_apps_from_roots(&roots))
        .unwrap_or_default();
    candidates_from_sources(
        steam_apps,
        enumerate_capturable_windows(),
        existing_custom_games,
        |path| game_icon::extract_exe_icon_data_url(path),
    )
}
```

Add registry/fallback root detection with `reg.exe`, mirroring the existing WebView2 registry style in `app.rs`:

```rust
fn steam_install_roots() -> Result<Vec<PathBuf>, String> {
    let mut roots = Vec::new();
    if let Some(path) = query_reg_sz(
        r"HKCU\Software\Valve\Steam",
        "SteamPath",
    ) {
        add_unique_path(&mut roots, PathBuf::from(path.replace('/', "\\")));
    }
    if let Some(program_files_x86) = std::env::var_os("ProgramFiles(x86)") {
        add_unique_path(&mut roots, PathBuf::from(program_files_x86).join("Steam"));
    }
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        add_unique_path(&mut roots, PathBuf::from(program_files).join("Steam"));
    }
    Ok(roots.into_iter().filter(|path| path.exists()).collect())
}

fn query_reg_sz(key: &str, value_name: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = std::process::Command::new("reg.exe")
        .args(["query", key, "/v", value_name])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_reg_sz_output(&String::from_utf8_lossy(&output.stdout), value_name)
}

fn parse_reg_sz_output(output: &str, value_name: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let name = fields.next()?;
        let kind = fields.next()?;
        if !name.eq_ignore_ascii_case(value_name) || !kind.eq_ignore_ascii_case("REG_SZ") {
            return None;
        }
        let value = fields.collect::<Vec<_>>().join(" ");
        (!value.is_empty()).then_some(value)
    })
}
```

Add candidate merge/filter/sort:

```rust
fn candidates_from_sources(
    steam_apps: Vec<SteamApp>,
    windows: Vec<CapturableWindow>,
    existing_custom_games: &[CustomGameSettings],
    icon_for_path: impl Fn(&str) -> Option<String>,
) -> Vec<DetectedGameCandidate> {
    let mut candidates = Vec::new();
    let mut consumed_windows = Vec::new();

    for app in steam_apps {
        let matched = windows
            .iter()
            .enumerate()
            .filter(|(_, window)| !is_noise_window(window))
            .find(|(_, window)| {
                window
                    .exe_path
                    .as_deref()
                    .is_some_and(|path| is_path_within(Path::new(path), &app.install_dir))
            });

        if let Some((index, window)) = matched {
            consumed_windows.push(index);
            let process_path = window.exe_path.clone().or_else(|| {
                app.process_path
                    .as_ref()
                    .map(|path| path.to_string_lossy().to_string())
            });
            candidates.push(DetectedGameCandidate {
                id_hint: format!("steam-{}", app.app_id),
                name: app.name,
                source: DetectedGameSource::SteamAndRunningWindow,
                steam_app_id: Some(app.app_id),
                install_dir: Some(app.install_dir.to_string_lossy().to_string()),
                exe_name: if window.exe_name.trim().is_empty() {
                    app.exe_name.unwrap_or_default()
                } else {
                    window.exe_name.clone()
                },
                process_path: process_path.clone(),
                window_title: window.title.clone(),
                icon: process_path.as_deref().and_then(&icon_for_path),
                confidence: 100,
            });
        } else if let (Some(exe_name), Some(process_path)) = (app.exe_name, app.process_path) {
            let path = process_path.to_string_lossy().to_string();
            candidates.push(DetectedGameCandidate {
                id_hint: format!("steam-{}", app.app_id),
                name: app.name,
                source: DetectedGameSource::Steam,
                steam_app_id: Some(app.app_id),
                install_dir: Some(app.install_dir.to_string_lossy().to_string()),
                exe_name,
                process_path: Some(path.clone()),
                window_title: String::new(),
                icon: icon_for_path(&path),
                confidence: 75,
            });
        }
    }

    for (index, window) in windows.into_iter().enumerate() {
        if consumed_windows.contains(&index) || is_noise_window(&window) {
            continue;
        }
        let process_path = window.exe_path.clone();
        candidates.push(DetectedGameCandidate {
            id_hint: format!("window-{}-{}", window.process_id, window.exe_name),
            name: game_name_from_window(&window),
            source: DetectedGameSource::RunningWindow,
            steam_app_id: None,
            install_dir: None,
            exe_name: window.exe_name.clone(),
            process_path: process_path.clone(),
            window_title: window.title.clone(),
            icon: process_path.as_deref().and_then(&icon_for_path),
            confidence: if process_path.is_some() { 90 } else { 65 },
        });
    }

    dedupe_candidates(candidates, existing_custom_games)
}
```

Add helpers:

```rust
fn dedupe_candidates(
    mut candidates: Vec<DetectedGameCandidate>,
    existing_custom_games: &[CustomGameSettings],
) -> Vec<DetectedGameCandidate> {
    candidates.retain(|candidate| has_match_identity(candidate));
    candidates.retain(|candidate| {
        !existing_custom_games.iter().any(|game| {
            candidate
                .process_path
                .as_deref()
                .zip(game.process_path.as_deref())
                .is_some_and(|(a, b)| normalize_path_string(a) == normalize_path_string(b))
                || (!candidate.exe_name.trim().is_empty()
                    && !game.exe_name.trim().is_empty()
                    && candidate.exe_name.eq_ignore_ascii_case(game.exe_name.trim()))
                || candidate.name.eq_ignore_ascii_case(game.name.trim())
        })
    });
    candidates.sort_by(|a, b| {
        sort_bucket(a)
            .cmp(&sort_bucket(b))
            .then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
    });

    let mut deduped: Vec<DetectedGameCandidate> = Vec::new();
    for candidate in candidates {
        if deduped.iter().any(|existing| same_candidate(existing, &candidate)) {
            continue;
        }
        deduped.push(candidate);
    }
    deduped
}

fn has_match_identity(candidate: &DetectedGameCandidate) -> bool {
    !candidate.exe_name.trim().is_empty()
        || candidate
            .process_path
            .as_deref()
            .is_some_and(|path| !path.trim().is_empty())
        || !candidate.window_title.trim().is_empty()
}

fn same_candidate(a: &DetectedGameCandidate, b: &DetectedGameCandidate) -> bool {
    a.process_path
        .as_deref()
        .zip(b.process_path.as_deref())
        .is_some_and(|(left, right)| normalize_path_string(left) == normalize_path_string(right))
        || (!a.exe_name.trim().is_empty()
            && !b.exe_name.trim().is_empty()
            && a.exe_name.eq_ignore_ascii_case(b.exe_name.trim()))
}

fn sort_bucket(candidate: &DetectedGameCandidate) -> u8 {
    match candidate.source {
        DetectedGameSource::RunningWindow if candidate.process_path.is_some() => 0,
        DetectedGameSource::SteamAndRunningWindow => 1,
        DetectedGameSource::Steam => 2,
        DetectedGameSource::RunningWindow => 3,
    }
}

fn is_path_within(path: &Path, parent: &Path) -> bool {
    normalize_path_string(&path.to_string_lossy())
        .starts_with(&normalize_path_string(&parent.to_string_lossy()))
}

fn normalize_path_string(path: &str) -> String {
    path.trim().replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase()
}

fn game_name_from_window(window: &CapturableWindow) -> String {
    let exe = window.exe_name.trim().trim_end_matches(".exe");
    if exe.is_empty() {
        window.title.trim().to_string()
    } else {
        exe.to_string()
    }
}

fn is_noise_window(window: &CapturableWindow) -> bool {
    let exe = window.exe_name.to_ascii_lowercase();
    let title = window.title.to_ascii_lowercase();
    is_helper_exe_name(&exe)
        || matches!(
            exe.as_str(),
            "steam.exe"
                | "steamwebhelper.exe"
                | "epicgameslauncher.exe"
                | "chrome.exe"
                | "msedge.exe"
                | "firefox.exe"
                | "brave.exe"
                | "opera.exe"
                | "discord.exe"
        )
        || title.contains("launcher")
        || title.contains("updater")
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run:

```powershell
cargo test -p clipline-app game_discovery::tests
```

Expected: PASS for all `game_discovery::tests`.

- [ ] **Step 5: Commit**

```powershell
git add apps/clipline-app/src/game_discovery.rs
git commit -m "feat(games): build detected game candidates"
```

---

### Task 4: Tauri Command Integration

**Files:**
- Modify: `apps/clipline-app/src/app.rs`
- Test: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write failing command registry test**

In `apps/clipline-app/tests/ui_contract.rs`, extend `games_ui_wires_detection_commands()` with backend command registry expectations:

```rust
for required in [
    "fn detect_installed_games",
    "detect_installed_games,",
] {
    assert!(
        app_rs().contains(required),
        "native command registry must expose detected game scan through {required}"
    );
}
```

Use the existing `app_rs()` helper in this file for backend command-registry assertions.

- [ ] **Step 2: Run test to verify it fails**

Run:

```powershell
cargo test -p clipline-app --test ui_contract games_ui_wires_detection_commands
```

Expected: FAIL because the backend command and native command registration do not exist.

- [ ] **Step 3: Add the command**

In `apps/clipline-app/src/app.rs`, import the new type and settings type:

```rust
use crate::game_discovery::DetectedGameCandidate;
use crate::settings::CustomGameSettings;
```

Add this command near `list_game_windows`:

```rust
#[tauri::command(async)]
fn detect_installed_games(
    existing_custom_games: Vec<CustomGameSettings>,
) -> Vec<DetectedGameCandidate> {
    crate::game_discovery::detect_installed_games(&existing_custom_games)
}
```

Register it in `tauri::generate_handler!` next to the existing game commands:

```rust
list_game_plugins,
list_game_windows,
detect_installed_games,
extract_window_icon,
```

- [ ] **Step 4: Run backend build check**

Run:

```powershell
cargo test -p clipline-app game_discovery::tests
```

Expected: PASS and no command signature compile errors.

- [ ] **Step 5: Commit**

```powershell
git add apps/clipline-app/src/app.rs apps/clipline-app/tests/ui_contract.rs
git commit -m "feat(games): expose detected game scan command"
```

---

### Task 5: Detection Panel Markup, CSS, and UI Contract

**Files:**
- Modify: `apps/clipline-app/ui/index.html`
- Modify: `apps/clipline-app/ui/styles.css`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write failing UI contract IDs test**

In the existing required HTML IDs list in `apps/clipline-app/tests/ui_contract.rs`, add:

```rust
"id=\"detect-games\"",
"id=\"detected-games-panel\"",
"id=\"detected-games-list\"",
"id=\"add-detected-games\"",
"id=\"cancel-detected-games\"",
```

Also extend `games_ui_wires_detection_commands()` with CSS/JS string expectations:

```rust
for required in [
    ".detected-game",
    ".detected-games-panel[hidden]",
] {
    assert!(
        styles_css().contains(required),
        "styles.css must style detected games workflow through {required}"
    );
}
```

- [ ] **Step 2: Run UI contract to verify it fails**

Run:

```powershell
cargo test -p clipline-app --test ui_contract games_ui_wires_detection_commands review_player_owns_all_controls
```

Expected: FAIL because the new IDs and CSS selectors do not exist.

- [ ] **Step 3: Add HTML controls**

In `apps/clipline-app/ui/index.html`, replace the single `Add Custom Game` button in the Custom games header with a button group:

```html
<div class="games-panel-actions">
  <button id="detect-games" type="button">Detect Games</button>
  <button id="add-custom-game" type="button">Add Custom Game</button>
</div>
```

Add this panel after `<div id="custom-games" class="custom-games"></div>` and before `game-window-picker`:

```html
<div id="detected-games-panel" class="detected-games-panel" hidden>
  <div class="game-window-picker-head">
    <strong>Detected games</strong>
    <div>
      <button id="add-detected-games" type="button" disabled>Add game(s)</button>
      <button id="cancel-detected-games" type="button">Cancel</button>
    </div>
  </div>
  <div id="detected-games-list" class="detected-games-list"></div>
</div>
```

- [ ] **Step 4: Add CSS**

In `apps/clipline-app/ui/styles.css`, extend existing selectors:

```css
.supported-games,
.custom-games,
.detected-games-panel,
.game-window-picker {
  display: grid;
  gap: 8px;
  min-width: 0;
}

.game-profile,
.custom-game,
.detected-game,
.game-window {
  display: grid;
  gap: 2px;
  min-width: 0;
  padding: 9px 10px;
  border: 1px solid var(--line);
  border-radius: 7px;
  background: #12151b;
}

.game-profile strong,
.custom-game strong,
.detected-game strong,
.game-window strong {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
  font-weight: 600;
}

.game-profile span,
.custom-game span,
.detected-game span,
.game-window span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--muted);
  font-size: 11.5px;
}
```

Add dedicated layout:

```css
.games-panel-actions,
.game-window-picker-head div {
  display: flex;
  gap: 8px;
}

.detected-games-panel {
  padding: 10px;
  border: 1px solid var(--line);
  border-radius: 7px;
  background: var(--panel-2);
}

.detected-games-panel[hidden] { display: none; }

.detected-games-list {
  display: grid;
  gap: 6px;
  max-height: 260px;
  min-height: 0;
  overflow-y: auto;
}

.detected-game {
  grid-template-columns: auto auto minmax(0, 1fr);
  align-items: center;
  column-gap: 10px;
  row-gap: 2px;
}

.detected-game .check-line {
  justify-content: center;
}

.detected-game-meta {
  display: grid;
  gap: 2px;
  min-width: 0;
}
```

- [ ] **Step 5: Run UI contract to verify it passes**

Run:

```powershell
cargo test -p clipline-app --test ui_contract games_ui_wires_detection_commands review_player_owns_all_controls
```

Expected: PASS for these tests.

- [ ] **Step 6: Commit**

```powershell
git add apps/clipline-app/ui/index.html apps/clipline-app/ui/styles.css apps/clipline-app/tests/ui_contract.rs
git commit -m "feat(ui): add detected games panel"
```

---

### Task 6: Frontend Scan, Selection, Dedupe, and Add Flow

**Files:**
- Modify: `apps/clipline-app/ui/app-core.js`
- Modify: `apps/clipline-app/ui/settings.js`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write failing frontend wiring contract**

Extend `games_ui_wires_detection_commands()` in `apps/clipline-app/tests/ui_contract.rs` with these required strings:

```rust
for required in [
    "var detectedGameCandidates = []",
    "var selectedDetectedGameIds = new Set()",
    "await invoke(\"detect_installed_games\", { existingCustomGames: customGames })",
    "renderDetectedGames",
    "showDetectedGamesPanel",
    "addSelectedDetectedGames",
    "$(\"detect-games\").addEventListener(\"click\", showDetectedGamesPanel)",
    "$(\"add-detected-games\").addEventListener(\"click\", addSelectedDetectedGames)",
    "$(\"cancel-detected-games\").addEventListener(\"click\", hideDetectedGamesPanel)",
] {
    assert!(
        js.contains(required),
        "main/settings JS must wire detected games workflow through {required}"
    );
}
```

- [ ] **Step 2: Run contract to verify it fails**

Run:

```powershell
cargo test -p clipline-app --test ui_contract games_ui_wires_detection_commands
```

Expected: FAIL because the frontend state/functions/listeners are missing.

- [ ] **Step 3: Add frontend state**

In `apps/clipline-app/ui/app-core.js`, add after `var gameWindows = [];`:

```js
var detectedGameCandidates = [];
var selectedDetectedGameIds = new Set();
```

- [ ] **Step 4: Add detected-game helpers and render function**

In `apps/clipline-app/ui/settings.js`, add near the custom game helpers:

```js
function detectedGameKey(candidate) {
  return String(candidate.id_hint || candidate.process_path || candidate.exe_name || candidate.name || "");
}

function detectedGameSourceLabel(candidate) {
  switch (candidate.source) {
    case "steam_and_running_window":
      return "Steam + running window";
    case "steam":
      return "Steam";
    case "running_window":
      return "Running window";
    default:
      return "Detected";
  }
}

function detectedGameMeta(candidate) {
  const parts = [detectedGameSourceLabel(candidate)];
  if (candidate.exe_name) parts.push(candidate.exe_name);
  if (candidate.window_title) parts.push(candidate.window_title);
  if (!candidate.window_title && candidate.install_dir) parts.push(candidate.install_dir);
  if (!candidate.window_title && !candidate.install_dir && candidate.steam_app_id) {
    parts.push(`Steam app ${candidate.steam_app_id}`);
  }
  return parts.join(" · ");
}

function customGameMatchesCandidate(game, candidate) {
  const gamePath = String(game.process_path || "").toLowerCase();
  const candidatePath = String(candidate.process_path || "").toLowerCase();
  if (gamePath && candidatePath && gamePath === candidatePath) return true;
  if (game.exe_name && candidate.exe_name && String(game.exe_name).toLowerCase() === String(candidate.exe_name).toLowerCase()) return true;
  return String(game.name || "").toLowerCase() === String(candidate.name || "").toLowerCase();
}

function renderDetectedGames() {
  const root = $("detected-games-list");
  root.replaceChildren();
  $("add-detected-games").disabled = selectedDetectedGameIds.size === 0;
  const addable = detectedGameCandidates.filter(
    (candidate) => !customGames.some((game) => customGameMatchesCandidate(game, candidate)),
  );
  if (!addable.length) {
    const empty = document.createElement("div");
    empty.className = "hint";
    empty.textContent = "no new games found";
    root.appendChild(empty);
    return;
  }
  for (const candidate of addable) {
    const key = detectedGameKey(candidate);
    const row = document.createElement("label");
    row.className = "detected-game";

    const check = document.createElement("span");
    check.className = "check-line";
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = selectedDetectedGameIds.has(key);
    checkbox.addEventListener("change", () => {
      if (checkbox.checked) {
        selectedDetectedGameIds.add(key);
      } else {
        selectedDetectedGameIds.delete(key);
      }
      $("add-detected-games").disabled = selectedDetectedGameIds.size === 0;
    });
    check.appendChild(checkbox);

    const icon = gameIconEl(candidate.icon, candidate.name);
    const meta = document.createElement("div");
    meta.className = "detected-game-meta";
    const name = document.createElement("strong");
    name.textContent = candidate.name || "Detected game";
    const info = document.createElement("span");
    info.textContent = detectedGameMeta(candidate);
    meta.append(name, info);
    row.append(check, icon, meta);
    root.appendChild(row);
  }
}
```

- [ ] **Step 5: Add scan, hide, conversion, and add-selected functions**

Still in `settings.js`, add near the running-window picker functions:

```js
async function showDetectedGamesPanel() {
  $("error").textContent = "";
  $("detected-games-panel").hidden = false;
  selectedDetectedGameIds = new Set();
  detectedGameCandidates = [];
  $("add-detected-games").disabled = true;
  $("detected-games-list").replaceChildren();
  const loading = document.createElement("div");
  loading.className = "hint";
  loading.textContent = "scanning installed and running games...";
  $("detected-games-list").appendChild(loading);
  try {
    detectedGameCandidates = await invoke("detect_installed_games", { existingCustomGames: customGames });
    renderDetectedGames();
  } catch (e) {
    $("error").textContent = e;
    detectedGameCandidates = [];
    renderDetectedGames();
  }
}

function hideDetectedGamesPanel() {
  $("detected-games-panel").hidden = true;
  detectedGameCandidates = [];
  selectedDetectedGameIds = new Set();
}

function customGameFromDetectedCandidate(candidate) {
  return normalizeCustomGame({
    id: customGameId(candidate.name),
    name: candidate.name || "Detected game",
    enabled: true,
    exe_name: candidate.exe_name || "",
    process_path: candidate.process_path || null,
    window_title: candidate.window_title || "",
    recording_mode: "replays_only",
    icon: candidate.icon || null,
  });
}

function addSelectedDetectedGames() {
  const selected = detectedGameCandidates.filter((candidate) =>
    selectedDetectedGameIds.has(detectedGameKey(candidate)),
  );
  const additions = selected
    .filter((candidate) => !customGames.some((game) => customGameMatchesCandidate(game, candidate)))
    .map(customGameFromDetectedCandidate);
  if (!additions.length) {
    renderDetectedGames();
    return;
  }
  customGames.push(...additions);
  hideDetectedGamesPanel();
  renderCustomGames();
  updateGameDetectionStatus();
  $("settings-status").textContent =
    additions.length === 1
      ? "custom game added - save to apply"
      : `${additions.length} custom games added - save to apply`;
}
```

- [ ] **Step 6: Wire listeners**

In `apps/clipline-app/ui/main.js`, add next to the existing custom game listener wiring:

```js
$("detect-games").addEventListener("click", showDetectedGamesPanel);
$("add-detected-games").addEventListener("click", addSelectedDetectedGames);
$("cancel-detected-games").addEventListener("click", hideDetectedGamesPanel);
```

- [ ] **Step 7: Run contract and focused app tests**

Run:

```powershell
cargo test -p clipline-app --test ui_contract games_ui_wires_detection_commands
cargo test -p clipline-app game_discovery::tests
```

Expected: PASS.

- [ ] **Step 8: Commit**

```powershell
git add apps/clipline-app/ui/app-core.js apps/clipline-app/ui/settings.js apps/clipline-app/ui/main.js apps/clipline-app/tests/ui_contract.rs
git commit -m "feat(ui): add detected games selection flow"
```

---

### Task 7: Final Verification, Handoff, and Live App Check

**Files:**
- Modify: `handoff.md`

- [ ] **Step 1: Run full workspace tests**

Run:

```powershell
cargo test --workspace
```

Expected: PASS.

- [ ] **Step 2: Run clippy**

Run:

```powershell
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: PASS with zero warnings.

- [ ] **Step 3: Update handoff**

Add a concise bullet to the completed/current state section of `handoff.md`:

```markdown
- Settings > Games now has a manual Detect Games workflow beside Add Custom Game. It scans Steam manifests and visible running windows, shows unchecked candidates, dedupes existing custom games, and appends selected rows as normal Custom games using the existing save-to-apply flow.
```

- [ ] **Step 4: Run docs-only status check**

Run:

```powershell
git diff -- handoff.md
```

Expected: Diff contains only the new Detect Games handoff note.

- [ ] **Step 5: Commit**

```powershell
git add handoff.md
git commit -m "docs: update handoff for detect games"
```

- [ ] **Step 6: Stop existing Clipline app processes before live run**

Run:

```powershell
Get-Process clipline-app -ErrorAction SilentlyContinue | Stop-Process
```

Expected: Existing `clipline-app.exe` processes are stopped; no error if none were running.

- [ ] **Step 7: Launch app for manual verification**

Run:

```powershell
cargo run -p clipline-app
```

Expected: Clipline opens. In Settings > Games, press Detect Games and verify:

- The button opens an inline Detected games panel below Custom games.
- Rows start unchecked.
- Add game(s) is disabled before selecting a row.
- A running Steam game appears with source `Steam + running window` when available.
- A running non-Steam game/window appears with source `Running window`.
- Selecting rows and pressing Add game(s) appends enabled Custom games with Replays only selected.
- Save applies the added Custom games.
- Pressing Detect Games again does not show the just-added games as new candidates.

- [ ] **Step 8: Final branch status**

Run:

```powershell
git status --short --branch
```

Expected: `codex/detect-games` is clean and ahead of `origin/main` by the feature commits.
