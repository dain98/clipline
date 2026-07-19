# L-09 Renderer Filesystem Authority Plan

> **Finding:** L-09 — renderer-controlled folder/icon inputs and recursive asset scopes amplify a future renderer compromise.

## Goal

Make every renderer-visible filesystem capability originate from a backend-observed user selection,
enumerated process, or validated media result, and expose only the exact files the webview needs.

## TDD sequence

- [ ] Add native-folder authorization tests proving unchanged roots remain valid, arbitrary changes
  fail, a native selection authorizes only the matching path, and failed/uncommitted saves can retry.
- [ ] Add media-root validation tests rejecting filesystem, Windows, program-data, and profile roots
  while permitting a normal nested media directory.
- [ ] Add local-executable path tests rejecting relative, UNC, device/verbatim-namespace, non-EXE,
  missing, and non-file paths before any Shell API call.
- [ ] Replace UI/runtime contracts that require recursive asset scopes or renderer-provided icon paths
  with exact-file and backend-enumeration contracts; run them red.
- [ ] Bind media-root changes to the latest native folder-picker result and commit authorization only
  after the settings transaction succeeds.
- [ ] Remove static and runtime recursive media/cache scopes; exact-scope validated local clips,
  generated posters/audio previews, and individual Cloud assets as they are returned.
- [ ] Resolve custom-game icon extraction by backend-enumerated process id, then validate the local
  executable inside the shared icon extractor before touching Shell APIs.
- [ ] Run focused tests, fresh-cache app Clippy, CI-mode workspace tests, and workspace Clippy with
  warnings denied.
- [ ] Rebuild/open Clipline and smoke Library playback/posters, native media selection cancellation,
  and custom-game window enumeration without changing persisted settings.
- [ ] Update `handoff.md`, the combined audit ledger, and the final manual acceptance checklist.

## Invariants

- [ ] Renderer IPC cannot change `media_dir` to a path not returned by the native picker.
- [ ] A media directory can never be a filesystem/drive root or an exact sensitive OS/profile root.
- [ ] The asset protocol receives exact validated files, never an entire media or Cloud-cache tree.
- [ ] Switching or resolving media roots does not accumulate recursive read capability.
- [ ] Renderer text never directly selects a filesystem path for Windows Shell icon extraction.
- [ ] UNC and device namespace icon paths are rejected even when supplied by a backend caller.
- [ ] Existing local Library, poster, audio-preview, and Cloud playback behavior remains intact.

## Commits

- `docs(plan): define L-09 filesystem authority`
- `fix(security): narrow renderer filesystem authority`
- `docs(audit): close renderer filesystem scope finding`
