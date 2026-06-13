//! Custom game detection. This layer only consumes visible window/process
//! metadata exposed by Win32; it never opens game memory or injects code.

use clipline_capture::windows::{enumerate_capturable_windows, CapturableWindow};

use crate::settings::{CustomGameSettings, GameSettings};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GameWindowInfo {
    pub title: String,
    pub process_id: u32,
    pub exe_name: String,
    pub exe_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DetectedGame {
    pub id: String,
    pub name: String,
    pub hwnd: isize,
    pub window_title: String,
    pub process_id: u32,
    pub exe_name: String,
}

pub fn list_game_windows() -> Vec<GameWindowInfo> {
    let current_pid = std::process::id();
    let mut windows: Vec<_> = enumerate_capturable_windows()
        .into_iter()
        .filter(|window| window.process_id != current_pid)
        .map(|window| GameWindowInfo {
            title: window.title,
            process_id: window.process_id,
            exe_name: window.exe_name,
            exe_path: window.exe_path,
        })
        .collect();
    windows.sort_by(|a, b| {
        a.exe_name
            .to_ascii_lowercase()
            .cmp(&b.exe_name.to_ascii_lowercase())
            .then_with(|| {
                a.title
                    .to_ascii_lowercase()
                    .cmp(&b.title.to_ascii_lowercase())
            })
    });
    windows
}

pub fn detect_active_game(settings: &GameSettings) -> Option<DetectedGame> {
    if !settings.auto_detect {
        return None;
    }
    detect_active_game_from_windows(settings, enumerate_capturable_windows())
}

pub fn detect_active_game_from_windows(
    settings: &GameSettings,
    windows: Vec<CapturableWindow>,
) -> Option<DetectedGame> {
    if !settings.auto_detect {
        return None;
    }
    for game in settings.custom_games.iter().filter(|game| game.enabled) {
        if let Some(window) = best_window_for_game(game, &windows) {
            return Some(DetectedGame {
                id: game.id.clone(),
                name: game.name.clone(),
                hwnd: window.handle,
                window_title: window.title.clone(),
                process_id: window.process_id,
                exe_name: window.exe_name.clone(),
            });
        }
    }
    None
}

fn best_window_for_game<'a>(
    game: &CustomGameSettings,
    windows: &'a [CapturableWindow],
) -> Option<&'a CapturableWindow> {
    windows
        .iter()
        .filter_map(|window| match_score(game, window).map(|score| (score, window)))
        .max_by_key(|(score, window)| (*score, window.title.len()))
        .map(|(_, window)| window)
}

fn match_score(game: &CustomGameSettings, window: &CapturableWindow) -> Option<u16> {
    let mut score = 0u16;
    if let (Some(configured), Some(actual)) = (&game.process_path, &window.exe_path) {
        if path_key(configured) == path_key(actual) {
            score = score.max(300);
        }
    }
    if !game.exe_name.trim().is_empty()
        && game.exe_name.eq_ignore_ascii_case(window.exe_name.trim())
    {
        score = score.max(200);
    }
    if !game.window_title.trim().is_empty()
        && contains_case_insensitive(&window.title, &game.window_title)
    {
        score = score.max(100);
    }
    (score > 0).then_some(score)
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.trim().to_ascii_lowercase())
}

fn path_key(path: &str) -> String {
    path.trim().replace('/', "\\").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> CustomGameSettings {
        CustomGameSettings {
            id: "custom-test".into(),
            name: "Test Game".into(),
            enabled: true,
            exe_name: "game.exe".into(),
            process_path: Some(r"C:\Games\Test\game.exe".into()),
            window_title: "Test Game".into(),
        }
    }

    fn window(
        handle: isize,
        title: &str,
        exe_name: &str,
        exe_path: Option<&str>,
    ) -> CapturableWindow {
        CapturableWindow {
            handle,
            title: title.into(),
            process_id: handle as u32,
            exe_name: exe_name.into(),
            exe_path: exe_path.map(str::to_string),
        }
    }

    #[test]
    fn detects_first_enabled_custom_game_by_process_path() {
        let settings = GameSettings {
            auto_detect: true,
            recording_mode: Default::default(),
            custom_games: vec![game()],
        };
        let detected = detect_active_game_from_windows(
            &settings,
            vec![window(
                42,
                "Unexpected title",
                "game.exe",
                Some(r"c:/games/test/GAME.exe"),
            )],
        )
        .expect("game should match by path");

        assert_eq!(detected.hwnd, 42);
        assert_eq!(detected.name, "Test Game");
    }

    #[test]
    fn falls_back_to_exe_name_when_path_is_unavailable() {
        let settings = GameSettings {
            auto_detect: true,
            recording_mode: Default::default(),
            custom_games: vec![game()],
        };
        let detected = detect_active_game_from_windows(
            &settings,
            vec![window(7, "Different title", "GAME.EXE", None)],
        )
        .expect("game should match by executable name");

        assert_eq!(detected.hwnd, 7);
    }

    #[test]
    fn disabled_or_global_off_games_do_not_match() {
        let disabled = CustomGameSettings {
            enabled: false,
            ..game()
        };
        let windows = vec![window(
            1,
            "Test Game",
            "game.exe",
            Some(r"C:\Games\Test\game.exe"),
        )];

        assert!(detect_active_game_from_windows(
            &GameSettings {
                auto_detect: true,
                recording_mode: Default::default(),
                custom_games: vec![disabled],
            },
            windows.clone(),
        )
        .is_none());
        assert!(detect_active_game_from_windows(
            &GameSettings {
                auto_detect: false,
                recording_mode: Default::default(),
                custom_games: vec![game()],
            },
            windows,
        )
        .is_none());
    }
}
