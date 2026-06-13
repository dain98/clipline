# Clipline Custom Game Detection

## Goal

Add a Settings > Games page that lets a user register a running game window as a
custom game. Clipline should then detect that game on future runs and automatically
switch recording to the matched game window while it is open, falling back to the
configured capture target when it is not.

## Constraints

- Detection must stay anti-cheat-safe: visible-window/process enumeration only,
  no injection, no memory reads, no game-process hooks.
- Saved custom games must survive process ID changes between launches.
- Matching should prefer executable path when Windows exposes it, then executable
  name, then the selected title as a weak fallback.
- Switching targets restarts the recorder service only when the active matched
  window changes.
- Manual capture settings remain the fallback target.

## Implementation Steps

- [ ] Add persisted game settings and validation for custom game rules.
- [ ] Add Win32 visible-window enumeration with process ID, executable name, and
  executable path when accessible.
- [ ] Add neutral matching logic that chooses the first enabled custom game with a
  visible window.
- [ ] Add a service capture source for a concrete window handle.
- [ ] Add a runtime detector loop that switches between detected game window and
  fallback capture target.
- [ ] Add Settings > Games UI with supported-game placeholders, custom game list,
  running-window picker, enable toggles, and remove actions.
- [ ] Verify with settings/unit/UI tests, workspace build checks, and a live app
  launch.

## Acceptance Tests

- Legacy settings load with game auto-detection enabled and no custom games.
- A saved custom game round-trips through settings JSON.
- Invalid custom games without an executable/title identity are rejected.
- Matching prefers process path and falls back to executable name/title when needed.
- The UI contract exposes the Games tab, custom list, add picker, and running
  window list controls.
- The app exposes a `list_game_windows` command and listens for detector events.
- When a configured game is found, status text shows the game override and the
  service captures the matched window handle.
