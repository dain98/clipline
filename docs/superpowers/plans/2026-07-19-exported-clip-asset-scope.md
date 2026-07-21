# Exported Clip Asset Scope Fix

**Goal:** Make a newly exported local trim playable immediately, without requiring a full Library
rescan or application restart.

## Root cause

`list_clips` grants each discovered MP4 to Tauri's per-file asset protocol scope. `export_clip`
creates a new MP4 and the frontend inserts it directly into the Library cache, but the command does
not grant the new path to that scope. Opening the new card therefore reaches the WebView before the
next `list_clips` scan and can surface media error 4 even when the MP4 itself is valid.

## Implementation

- [ ] Add a failing command-contract test requiring `export_clip` to grant its completed target via
      the same local-clip asset helper used by `list_clips`.
- [ ] Give `export_clip` an `AppHandle`, preserve the configured media root, and grant the exported
      MP4 only after the blocking file transform completes successfully.
- [ ] Run the focused app contract tests, workspace tests, and warning-denied workspace Clippy.
- [ ] Record the acceptance failure, root cause, fix, and focused retest in `handoff.md` and the
      combined remediation program's manual checklist.

## Manual retest

1. Export a trim from the large session.
2. Open the newly inserted trim card immediately, without refreshing the Library or restarting.
3. Confirm metadata loads, playback starts, seeking works, and reopening the card still works.
4. Confirm the original session remains intact and playable.
