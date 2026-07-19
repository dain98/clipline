# Bulk Select Completion TDD Plan

## Goal

Remove the redundant bulk-selection exit control and automatically leave multi-select mode after a confirmed bulk delete removes at least one clip.

## Scope

- Keep the `Select multiple` toolbar control and its active `Done` label as the single manual exit.
- Remove the redundant bulk action bar `Cancel` button and listener.
- After a successful bulk delete reports one or more deleted clips, clear selection and exit select mode.
- Preserve the existing error and zero-delete behavior.

## TDD steps

- [ ] Update the UI contract test to reject the redundant `bulk-cancel` control and require the bulk-delete success path to call `exitSelectMode` when clips were deleted.
- [ ] Run the focused contract test and confirm it fails against the current implementation.
- [ ] Remove the redundant markup/listener and make the smallest bulk-delete state transition change.
- [ ] Run the focused contract test until green.
- [ ] Run workspace tests, formatting, clippy with warnings denied, and diff checks.
- [ ] Relaunch Clipline for user verification.
