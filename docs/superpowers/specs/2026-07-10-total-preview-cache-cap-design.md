# Total Audio Preview Cache Cap

## Problem

Live verification produced a 3.64 GiB audio-preview cache after generating a
1.95 GiB active preview. The existing pruning policy excludes protected files
from the byte total, so it independently permits up to 2 GiB of unprotected
files in addition to the protected preview. That contradicts the product
handoff and acceptance rule that the cache is capped at 2 GiB unless the active
preview alone is larger.

## Considered approaches

1. **Cap total physical preview bytes (selected).** Count protected and
   unprotected preview MP4s toward the 2 GiB limit, but evict only unprotected
   candidates. This matches the user-visible storage promise.
2. **Cap only reusable/unprotected bytes (current behavior).** This preserves
   the original Task 6 wording but allows total storage to approach 4 GiB with
   one protected preview, or more if multiple paths are protected.
3. **Reserve estimated headroom before generation.** This could reduce
   transient growth but requires predicting output size and still needs the
   total-byte policy after generation.

## Approved behavior

`prune_audio_preview_cache` counts every matching `audio-preview-*.mp4` in the
cache directory toward total preview bytes. Protected paths remain ineligible
for eviction. Oldest unprotected candidates are removed until total bytes are
at or below `AUDIO_PREVIEW_CACHE_MAX_BYTES`.

If protected previews alone exceed the limit, all unprotected candidates are
removed and the protected excess is retained. A protected file is never
deleted merely to satisfy the cap.

The prune report's `reusable_bytes` field continues to report the bytes left in
unprotected reusable previews, preserving its existing meaning. The eviction
loop uses a separate total-byte accumulator.

## Scope and error handling

Only the neutral cache policy and its focused filesystem tests change. Startup,
cache-hit, and post-generation call sites remain unchanged. Partial-file
cleanup, LRU ordering, canonical protected-path matching, best-effort removal,
and path-context error reporting remain unchanged.

## Tests

Add RED-first byte-sized filesystem cases proving:

- a protected file smaller than the cap consumes capacity and forces the
  oldest unprotected file out;
- a protected file larger than the cap is retained while every unprotected
  preview is evicted;
- `reusable_bytes` reports only surviving unprotected bytes.

Then run the focused cache tests, the full app suite, and Clippy with warnings
denied. Final live verification must restart the fixed app and confirm the real
cache is at or below 2 GiB unless the active preview alone is larger.
