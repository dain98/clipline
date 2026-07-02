#![allow(dead_code)]

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectedGameSource {
    Steam,
    RunningWindow,
    SteamAndRunningWindow,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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
    pub app_id: u32,
    pub name: String,
    pub install_dir_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SteamApp {
    app_id: u32,
    name: String,
    install_dir: PathBuf,
    exe_name: Option<String>,
    process_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum VdfEntry {
    Pair { key: String, value: String },
    Object { key: String, entries: Vec<VdfEntry> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum VdfToken {
    String(String),
    Open,
    Close,
}

fn parse_vdf(input: &str) -> Result<Vec<VdfEntry>, String> {
    let tokens = tokenize_vdf(input)?;
    let mut index = 0;
    parse_vdf_entries(&tokens, &mut index, false)
}

fn tokenize_vdf(input: &str) -> Result<Vec<VdfToken>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.char_indices().peekable();
    while let Some((offset, ch)) = chars.next() {
        match ch {
            '"' => {
                let mut value = String::new();
                let mut terminated = false;
                while let Some((_, string_ch)) = chars.next() {
                    match string_ch {
                        '"' => {
                            terminated = true;
                            break;
                        }
                        '\\' => {
                            let Some((_, escaped)) = chars.next() else {
                                return Err("unterminated string".into());
                            };
                            value.push(match escaped {
                                '\\' => '\\',
                                '"' => '"',
                                'n' => '\n',
                                'r' => '\r',
                                't' => '\t',
                                other => other,
                            });
                        }
                        other => value.push(other),
                    }
                }
                if !terminated {
                    return Err("unterminated string".into());
                }
                tokens.push(VdfToken::String(value));
            }
            '{' => tokens.push(VdfToken::Open),
            '}' => tokens.push(VdfToken::Close),
            '/' if chars.peek().is_some_and(|(_, next)| *next == '/') => {
                for (_, comment_ch) in chars.by_ref() {
                    if comment_ch == '\n' {
                        break;
                    }
                }
            }
            ch if ch.is_whitespace() => {}
            other => {
                return Err(format!(
                    "unexpected VDF character {other:?} at byte {offset}"
                ));
            }
        }
    }
    Ok(tokens)
}

fn parse_vdf_entries(
    tokens: &[VdfToken],
    index: &mut usize,
    in_object: bool,
) -> Result<Vec<VdfEntry>, String> {
    let mut entries = Vec::new();
    while let Some(token) = tokens.get(*index) {
        match token {
            VdfToken::Close => {
                *index += 1;
                if in_object {
                    return Ok(entries);
                }
                return Err("unexpected object close".into());
            }
            VdfToken::Open => return Err("unexpected object open".into()),
            VdfToken::String(key) => {
                let key = key.clone();
                *index += 1;
                let Some(value_token) = tokens.get(*index) else {
                    return Err(format!("missing value for key {key:?}"));
                };
                match value_token {
                    VdfToken::String(value) => {
                        entries.push(VdfEntry::Pair {
                            key,
                            value: value.clone(),
                        });
                        *index += 1;
                    }
                    VdfToken::Open => {
                        *index += 1;
                        let object_entries = parse_vdf_entries(tokens, index, true)?;
                        entries.push(VdfEntry::Object {
                            key,
                            entries: object_entries,
                        });
                    }
                    VdfToken::Close => return Err(format!("missing value for key {key:?}")),
                }
            }
        }
    }
    if in_object {
        Err("unterminated object".into())
    } else {
        Ok(entries)
    }
}

fn library_paths_from_vdf(entries: &[VdfEntry]) -> Vec<PathBuf> {
    let Some(libraryfolders) = find_object(entries, "libraryfolders") else {
        return Vec::new();
    };
    libraryfolders
        .iter()
        .filter_map(|entry| match entry {
            VdfEntry::Pair { value, .. } => Some(PathBuf::from(value)),
            VdfEntry::Object { entries, .. } => find_pair(entries, "path").map(PathBuf::from),
        })
        .collect()
}

fn steam_app_from_manifest(entries: &[VdfEntry]) -> Option<SteamAppManifest> {
    let app_state = find_object(entries, "AppState")?;
    Some(SteamAppManifest {
        app_id: find_pair(app_state, "appid")?.parse().ok()?,
        name: find_pair(app_state, "name")?.to_owned(),
        install_dir_name: find_pair(app_state, "installdir")?.to_owned(),
    })
}

fn steam_apps_from_roots(steam_roots: &[PathBuf]) -> Result<Vec<SteamApp>, String> {
    let mut libraries = Vec::new();
    for root in steam_roots {
        for library in steam_libraries_from_root(root)? {
            add_unique_path(&mut libraries, library);
        }
    }

    let mut apps = Vec::new();
    for library in libraries {
        let steamapps_dir = library.join("steamapps");
        let Ok(entries) = std::fs::read_dir(&steamapps_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !file_name.starts_with("appmanifest_") || !file_name.ends_with(".acf") {
                continue;
            }
            let Ok(contents) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Some(manifest) = parse_vdf(&contents)
                .ok()
                .and_then(|entries| steam_app_from_manifest(&entries))
            else {
                continue;
            };
            let install_dir = steamapps_dir
                .join("common")
                .join(&manifest.install_dir_name);
            let process_path = infer_executable_path(&install_dir, &manifest.name);
            let exe_name = process_path
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .map(str::to_owned);
            apps.push(SteamApp {
                app_id: manifest.app_id,
                name: manifest.name,
                install_dir,
                exe_name,
                process_path,
            });
        }
    }

    apps.sort_by_key(|app| app.name.to_lowercase());
    Ok(apps)
}

fn steam_libraries_from_root(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut libraries = vec![root.to_path_buf()];
    let libraryfolders = root.join("steamapps").join("libraryfolders.vdf");
    let contents = match std::fs::read_to_string(&libraryfolders) {
        Ok(contents) => contents,
        Err(_) => return Ok(libraries),
    };
    let Ok(entries) = parse_vdf(&contents) else {
        return Ok(libraries);
    };
    for library in library_paths_from_vdf(&entries) {
        add_unique_path(&mut libraries, library);
    }
    Ok(libraries)
}

fn add_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    let key = path_key(&path);
    if !paths.iter().any(|existing| path_key(existing) == key) {
        paths.push(path);
    }
}

fn infer_executable_path(install_dir: &Path, game_name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    collect_exe_candidates(install_dir, 0, &mut candidates);
    candidates.sort_by_key(|path| path_key(path));
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
        if out.len() >= 200 {
            return;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_exe_candidates(&path, depth + 1, out);
            continue;
        }
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        {
            out.push(path);
        }
    }
}

fn executable_score(path: &Path, install_dir: &Path, game_name: &str) -> u16 {
    let mut score = 10;
    let exe_key = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(normalized_name_key)
        .unwrap_or_default();
    let game_key = normalized_name_key(game_name);
    let install_key = install_dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(normalized_name_key)
        .unwrap_or_default();

    if !game_key.is_empty() && exe_key == game_key {
        score += 80;
    }
    if !install_key.is_empty() && exe_key == install_key {
        score += 70;
    }
    if !game_key.is_empty() && (exe_key.contains(&game_key) || game_key.contains(&exe_key)) {
        score += 35;
    }
    if path.parent().is_some_and(|parent| parent == install_dir) {
        score += 20;
    }
    score
}

fn is_helper_exe_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "crash",
        "crashhandler",
        "unitycrashhandler",
        "redist",
        "redistributable",
        "vcredist",
        "dxsetup",
        "setup",
        "install",
        "unins",
        "uninstall",
        "updater",
        "launcherhelper",
        "helper",
    ]
    .iter()
    .any(|helper| name.contains(helper))
}

fn normalized_name_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

fn find_object<'a>(entries: &'a [VdfEntry], key: &str) -> Option<&'a [VdfEntry]> {
    entries.iter().find_map(|entry| match entry {
        VdfEntry::Object {
            key: entry_key,
            entries,
        } if entry_key.eq_ignore_ascii_case(key) => Some(entries.as_slice()),
        _ => None,
    })
}

fn find_pair<'a>(entries: &'a [VdfEntry], key: &str) -> Option<&'a str> {
    entries.iter().find_map(|entry| match entry {
        VdfEntry::Pair {
            key: entry_key,
            value,
        } if entry_key.eq_ignore_ascii_case(key) => Some(value.as_str()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clipline_test_utils::TestDir;

    use super::*;

    fn vdf_path(path: &std::path::Path) -> String {
        path.to_string_lossy().replace('\\', "\\\\")
    }

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

    #[test]
    fn executable_score_adds_contains_bonus_for_exact_game_name() {
        let install_dir = PathBuf::from(r"C:\Games\InstallFolder");
        let exe_path = install_dir.join("Game Name.exe");

        assert_eq!(executable_score(&exe_path, &install_dir, "Game Name"), 145);
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
        std::fs::write(
            library.join("steamapps/common/SlayTheSpire/UnityCrashHandler64.exe"),
            b"",
        )
        .unwrap();
        std::fs::write(
            library.join("steamapps/common/SlayTheSpire/SlayTheSpire.exe"),
            b"",
        )
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
            format!(
                r#""libraryfolders" {{ "0" {{ "path" "{}" }} }}"#,
                vdf_path(&steam_root)
            ),
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
        assert_eq!(
            apps.iter().map(|app| app.name.as_str()).collect::<Vec<_>>(),
            vec!["Factorio"]
        );
    }

    #[test]
    fn steam_catalog_scans_root_when_libraryfolders_is_malformed() {
        let dir = TestDir::new("clipline-game-discovery", "steam-bad-libraryfolders");
        let steam_root = dir.path().join("Steam");
        let app_dir = steam_root.join("steamapps/common/Hades");
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::write(
            steam_root.join("steamapps/libraryfolders.vdf"),
            r#""libraryfolders" { "0" { "path" "#,
        )
        .unwrap();
        std::fs::write(
            steam_root.join("steamapps/appmanifest_1145360.acf"),
            r#""AppState" { "appid" "1145360" "name" "Hades" "installdir" "Hades" }"#,
        )
        .unwrap();
        std::fs::write(app_dir.join("Hades.exe"), b"").unwrap();

        let apps = steam_apps_from_roots(&[steam_root]).expect("steam scan");
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "Hades");
        assert_eq!(apps[0].exe_name.as_deref(), Some("Hades.exe"));
    }
}
