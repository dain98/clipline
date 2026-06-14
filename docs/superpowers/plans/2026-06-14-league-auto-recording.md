# 2026-06-14 League Auto Recording

Goal: introduce a first-class game plugin layer, then implement League of
Legends as the first built-in plugin. When the actual in-game League
process/window is present, Clipline should switch capture to that window and
start a full-session recording; when it disappears, the session should finalize
and Clipline should fall back to the configured capture target.

Plan:

- Add persisted plugin settings under `games.plugins.<plugin_id>`, with generic
  enabled/recording-mode fields so new games do not require settings schema
  changes.
- Add a backend game plugin catalog that exposes plugin metadata to the UI and
  owns per-game detection logic.
- Implement the `league_of_legends` plugin by matching `League of Legends.exe`
  as the in-game process while ignoring Riot launcher/client executables.
- Keep custom games working as they do now, with enabled plugins taking priority
  over generic custom rules.
- Ensure active plugin game state is cleared when that plugin or global game
  detection is disabled.
- Restart the recorder when an active game's recording mode changes, not just
  when its window handle changes.
- Replace the hardcoded supported-games placeholders with plugin rows rendered
  from backend metadata, including an enable toggle and recording-mode segmented
  control for each plugin.
- Cover the path with settings, detector, app-state, pure UI, and UI-contract
  tests, then run the app for manual testing.
