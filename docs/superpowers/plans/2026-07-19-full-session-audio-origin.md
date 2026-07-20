# Full-Session Audio-Origin Finalization Fix

**Goal:** Finalize full-session recordings when the first Opus packet straddles the first video
timestamp, while preserving Clipline's established rule that engine-start audio lead-in is omitted.

## Root cause

The recorder defines the first encoded video packet as the media timeline origin. Startup cleanup
currently removes only audio packets that end before that origin, so an indivisible 20 ms Opus
packet that starts before the origin and ends after it survives. The full-session writer then
correctly rejects its negative relative timestamp and preserves the `.mp4.recording` for recovery.

## Implementation

- [ ] Add a failing deterministic full-session test with a first video timestamp inside an Opus
      packet rather than on a packet boundary.
- [ ] Centralize startup-audio filtering and retain only packets whose start is at or after the
      video origin, allowing the existing sub-nanosecond tolerance.
- [ ] Prove replay coverage remains within one Opus packet and full-session finalization succeeds.
- [ ] Run focused capture tests, workspace tests, warning-denied workspace Clippy, and clean-cache
      capture Clippy.
- [ ] Update the handoff and manual acceptance record with the preserved-file symptom and retest.

## Manual retest

1. Start and finish a full-session recording with system or microphone audio enabled.
2. Confirm no `media sample timestamp precedes recording origin` warning appears.
3. Confirm the session is published as `.mp4`, appears in Library, and plays from the beginning.
4. Confirm any previously preserved `.mp4.recording` remains untouched for explicit recovery.
