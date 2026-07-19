# M-18 Clipboard Ownership Plan

**Goal:** Publish `CF_HDROP` through Clipline's real window owner, replace the clipboard only after ownership is established, and keep allocation ownership exact on every failure path.

## Win32 transaction contract

- [ ] Inject the invoking `WebviewWindow` into the Tauri command and resolve its native HWND before entering the blocking export task.
- [ ] Retry `OpenClipboard(owner)` for a short bounded interval when another process temporarily owns the clipboard.
- [ ] Call `EmptyClipboard` only after a successful open, then call `SetClipboardData(CF_HDROP, handle)` while the clipboard remains open.
- [ ] Transfer the movable allocation only after `SetClipboardData` succeeds; free it exactly once on open, empty, or set failure and close the clipboard on every opened path.

## Regression coverage

- [ ] Add a deterministic retry/state-order test covering busy-open recovery, terminal open failure, empty failure, set failure, and success.
- [ ] Keep the existing DROPFILES payload and verbatim-UNC normalization tests green.
- [ ] Add a UI/native command contract proving the invoking window remains an injected argument rather than renderer-controlled data.

## Verification and handoff

- [ ] Run focused app tests, fresh-cache app Clippy, CI-mode workspace tests, and workspace Clippy with warnings denied.
- [ ] Rebuild/open Clipline and perform a real copy/paste file-list smoke test using the app window owner.
- [ ] Record M-18 evidence in the audit ledger and `handoff.md`; add one final manual acceptance item for clipboard contention/real destination applications.
