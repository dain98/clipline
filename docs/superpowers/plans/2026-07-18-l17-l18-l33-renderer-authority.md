# L-17/L-18/L-33 Renderer Authority Plan

> **Findings:** L-17 — renderer-selected cloud origins; L-18 — inherited marker keys and unsafe
> CSS URL construction; L-33 — unused native window capabilities.

## Goal

Reduce the renderer boundary to opaque cloud clip identity, explicitly validated presentation
values, and only the Tauri window operations the current UI actually uses. Preserve configured
private/public Cloud deployments, unknown-marker fallbacks, bundled marker art, and custom
titlebar behavior.

## TDD sequence

- [ ] Add native tests proving a clip page URL is constructed from connected Cloud settings and an
  escaped/validated remote clip id, while empty/path-like ids and renderer-selected origins fail;
  run them red against the URL-taking command.
- [ ] Add DOM-free player tests proving inherited marker/category keys are ignored and marker art
  accepts only the bundled marker path/data-image forms without CSS delimiter characters; run red.
- [ ] Add a UI contract requiring the opaque remote id command payload, shared safe marker lookup,
  and the exact least-privilege window capability set; run red.
- [ ] Replace `open_cloud_clip_url(url)` with a state-owning `open_cloud_clip(remote_clip_id)` that
  validates the id, constructs the page with URL path-segment APIs, and launches only the saved
  public/host origin.
- [ ] Centralize own-property marker configuration and safe image validation in DOM-free
  `player-core.js`; use it from gallery and review rendering and retain fallbacks for unknown kinds.
- [ ] Remove unused minimize/maximize/unmaximize grants while retaining toggle-maximize, close,
  dragging, and resize-dragging permissions observed in the custom titlebar.
- [ ] Run focused native/player/UI tests, fresh-cache app Clippy, CI-mode workspace tests, and
  warning-denied workspace Clippy.
- [ ] Rebuild/open the native app and exercise minimize, maximize/restore, drag, close-to-tray,
  Library marker rendering, and the configured Cloud link where local state permits.
- [ ] Update `handoff.md`, the master ledger, and the cumulative manual checklist only for behavior
  that cannot be completed locally without a real Cloud account.

## Invariants

- [ ] No Tauri command accepts a renderer-provided external URL for cloud clip opening.
- [ ] Both configured `public_url` and `host_url` deployments remain valid, but no other origin is
  launchable through the command.
- [ ] Remote ids become one encoded URL path segment and cannot change path/origin/query semantics.
- [ ] `constructor`, `__proto__`, and other inherited keys never resolve presentation data.
- [ ] A marker image reaches CSS only if it is a simple bundled marker PNG path or canonical PNG
  data URL containing no CSS string delimiters.
- [ ] Unknown or invalid marker kinds/images fall back to the existing category glyphs.
- [ ] Renderer window permissions equal the operations invoked by the shipped UI.

## Commits

- `docs(plan): define renderer authority boundaries`
- `fix(security): narrow renderer-controlled actions`
- `docs(audit): close renderer authority findings`
