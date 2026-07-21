# L-05 Multipart Part Validation Plan

> **Finding:** L-05 — multipart part zero aliases part one and malformed missing-part lists are not validated as a set.

## Goal

Reject malformed server multipart work lists before reading or transmitting any file bytes, with one
shared validator used by proxy and direct-object upload paths.

## TDD sequence

- [ ] Add focused fixtures for part zero, duplicate parts, a part beyond the file-derived range, and
  a valid complete subset.
- [ ] Run the focused tests and record the expected compile/behavior failure.
- [ ] Validate positive bounded part size, derive the representable part range from file size, and
  require every missing part to be unique and within that range.
- [ ] Invoke the validator before both proxy and direct multipart loops; retain defensive per-part
  bounds in the file reader.
- [ ] Run focused tests, fresh-cache app Clippy, CI-mode workspace tests, and workspace Clippy with
  warnings denied.
- [ ] Update `handoff.md` and the combined audit ledger; add no manual test unless a real service
  behavior remains uncovered.

## Invariants

- [ ] Part zero is rejected before offset arithmetic, URL construction, file reads, or network I/O.
- [ ] Duplicate missing-part entries cannot upload or acknowledge one chunk more than once.
- [ ] A part whose start is at or beyond EOF is rejected before transfer.
- [ ] Both authenticated proxy chunks and direct object-store chunks consume the same validated list.
- [ ] Conforming resumable lists keep their server-provided order.

## Commits

- `docs(plan): define L-05 multipart validation`
- `fix(cloud): validate multipart work lists`
- `docs(audit): close multipart part validation finding`
