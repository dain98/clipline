---
include:
  - "crates/clipline-events/src/**"
  - "crates/clipline-lol/src/**"
  - "crates/clipline-storage/src/**"
---

Event marker and storage changes must preserve local-first privacy and deterministic clip behavior.

Review for:

- League Live Client Data polling that retries quietly outside matches and handles localhost HTTPS, monotonic `EventID` de-duplication, local-player identification, and undocumented events defensively.
- Game-clock to recording-time mapping that handles drift, game pauses, duplicate events, and clip-window rebasing without remapping already-written markers.
- Storage code that validates media roots and clip paths, prevents traversal, accounts for marker sidecars, protects the just-saved clip during quota GC, and keeps library/delete/export/quota behavior on the same configured media root.
