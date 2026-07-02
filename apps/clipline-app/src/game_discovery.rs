#![allow(dead_code)]

use std::path::PathBuf;

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
