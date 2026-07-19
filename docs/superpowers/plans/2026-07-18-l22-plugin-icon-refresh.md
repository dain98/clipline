# L-22 Plugin Icon Refresh Plan

**Goal:** Let a supported-game icon appear in the current process immediately after executable
extraction succeeds, without rebuilding immutable profile presentation data on every UI render.

**Finding:** `CODEBASE_AUDIT_COMBINED.md` L-22.

## Design boundary

- [ ] Keep parsed manifests and resolved immutable catalog fields in `OnceLock` storage.
- [ ] Return an owned catalog snapshot per command call and overlay only extracted-icon values from
      the current cache file, so an earlier missing file is never memoized permanently.
- [ ] Treat both an explicit `extracted` icon and a manifest with no bundled icon as extraction-backed
      when resolving the cache, matching the existing extraction eligibility rule.
- [ ] Reload the plugin catalog in the renderer when a newly active built-in game is detected; icon
      extraction occurs synchronously before that detection event is emitted.
- [ ] Keep icon file reads at the catalog command/change boundary, not in card/settings rendering.

## TDD sequence

- [ ] Add a cache-file resolver test proving a missing icon returns none and a file created later is
      returned as a PNG data URL in the same process.
- [ ] Replace the pointer-identity catalog assertion with tests that immutable catalog data remains
      stable while each dynamic catalog request is an independent owned snapshot.
- [ ] Add a UI contract proving active game detection refreshes the plugin catalog.
- [ ] Implement immutable base caching plus dynamic icon overlay and detection-triggered refresh.

## Verification

- [ ] Run focused game-plugin and UI-contract tests.
- [ ] Clean `clipline-app`, then run warning-denied app Clippy.
- [ ] Run CI-mode workspace tests and warning-denied workspace Clippy.
- [ ] Rebuild and open the exact workspace app; verify bundled supported-game icons and the normal
      nine-clip Library remain healthy.
- [ ] Update `handoff.md` and the combined remediation ledger. Add a manual item only if executable
      icon extraction itself cannot be covered by existing platform tests.
