# M-02 Transactional Settings and Credentials Plan

**Goal:** Keep settings.json, live runtime settings, Windows registrations, and Credential Manager references mutually recoverable across every fallible commit step.

## Backend settings staging

- [ ] Add injected-save tests proving failed Cloud and osu! settings writes leave live memory unchanged.
- [ ] Stage normalized Cloud/osu! changes on a clone, persist under the shared settings-save lock, and publish only after persistence succeeds.
- [ ] Preserve concurrent backend-owned fields while a frontend settings save is staged.

## Windows settings side effects

- [ ] Add failure-injection tests for multiple shortcut removals, proving earlier removals are re-registered when a later removal fails.
- [ ] Track every added and removed shortcut and surface rollback failures instead of silently leaving a mixed registration set.
- [ ] Compensate autostart and hotkey/runtime mutations when persistence or a later commit step fails; keep the old durable settings recoverable until the runtime commit finishes.
- [ ] Treat asset-scope expansion as an explicit harmless superset and perform all validation/path creation before side effects.

## Credential transactions

- [ ] Extract deterministic credential commit helpers with injected read/write/delete/persist operations.
- [ ] On Cloud connect or osu! credential replacement, restore a prior target value (or delete a newly created target) if settings persistence fails.
- [ ] On disconnect/removal, keep the durable reference unless deletion succeeds, and restore the credential if clearing the reference fails.
- [ ] Persist cleanup intent for an obsolete credential until deletion succeeds, so a partial cleanup is retryable rather than silently orphaned.
- [ ] Apply the same ordering to osu! connection-test identity migration.

## Verification

- [ ] Run focused app/settings/cloud/osu tests and fresh-cache app Clippy.
- [ ] Run CI-mode workspace tests and workspace Clippy with warnings denied.
- [ ] Rebuild/open Clipline for a native smoke test.
- [ ] Update `handoff.md`, the master audit ledger, and manual acceptance checks for release autostart/hotkey and real Credential Manager behavior.
