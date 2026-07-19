# MP4 Untrusted-Input Hardening

## Goal

Reject malformed or oversized MP4 metadata deterministically instead of overflowing box offsets,
allocating from attacker-declared sample counts, or emitting fragments with truncated box sizes or
`trun` data offsets.

## Plan

- [ ] Add failing walker regressions for overflowing large-size boxes and overflowing parent
      bounds, while preserving the existing stop-at-truncation behavior.
- [ ] Replace every box-walker offset addition with checked arithmetic and add a checked `box_end`
      conversion for trim parsing.
- [ ] Add failing sample-table regressions proving truncated entry counts and compressed `stts` /
      fixed-size `stsz` counts are rejected before large allocations.
- [ ] Validate table entry counts against their containing box, bound expanded sample counts, and
      parse durations against the already-validated `stsz` sample count.
- [ ] Add failing fragment regressions for large `mdat` headers, oversized plain boxes, and
      `trun` offsets beyond the signed 32-bit field.
- [ ] Use large-size `mdat` headers, checked size conversions, and fallible fragment offset
      construction throughout the writer paths.
- [ ] Run focused MP4 tests, workspace tests, formatting, fresh-cache MP4 clippy, and workspace
      clippy with warnings denied; then update `handoff.md` with the completed hardening.
