# H-05 Large-File Streaming Plan

**Goal:** Remove whole-MP4 memory materialization and redundant body copies from trim, audio-selection export, clipboard sharing, and cloud upload while preserving current output and resumable-upload behavior. Close M-16 in the shared file writer so source identity and existing targets remain safe on every failure.

## Batch 1: bounded MP4 file transforms

- [ ] Add regression coverage proving file transforms read only the bounded finalized `moov` metadata and stream media samples from the source file.
- [ ] Add file-backed selected-track remux and mixed-track export APIs. Keep only bounded per-packet decode/encode buffers in memory; spool mixed audio packets to a unique temporary file before the final mux.
- [ ] Route trim through the same metadata-only parser.
- [ ] Compare source and target file identities, including distinct hard links, before writing.
- [ ] Write every final transform to a unique `create_new` sibling temporary file, flush/sync it, and publish only after success. Clean owned temporaries on all errors and preserve an existing target.
- [ ] Route clipboard audio-selection exports through file-backed APIs instead of `read` + `Vec<u8>`.

## Batch 2: file-backed cloud payloads

- [ ] Add tests for streaming SHA-256, bounded part reads, single-upload streaming, resume seeks, direct-upload retry, and temporary audio-selection cleanup.
- [ ] Represent an upload payload as a path plus size/checksum rather than `Vec<u8>`.
- [ ] Hash with a fixed-size buffer and derive duration from bounded MP4 metadata/markers.
- [ ] Stream single PUT bodies from a file and read chunked/proxy/direct parts into one bounded reusable part buffer; do not clone whole bodies.
- [ ] Export selected/mixed audio to an owned temporary file and remove it after success or failure. Original uploads use the source path directly.
- [ ] Reject hostile/unsupported server part sizes before allocation with a clear error.

## Verification

- [ ] Run focused `clipline-mp4` and `clipline-app` tests after each red/green step.
- [ ] Run fresh-cache Clippy for both changed crates.
- [ ] Run workspace tests and workspace Clippy with warnings denied.
- [ ] Rebuild and open Clipline for a native smoke test.
- [ ] Update `handoff.md`, the combined remediation ledger, and the accumulated manual acceptance checklist.

## Manual acceptance additions

- Export a trim from a multi-gigabyte/full-session clip and confirm Clipline memory stays broadly flat, the source remains playable, and the completed export plays through its end.
- Copy a clip with one selected audio track and with multiple tracks mixed; paste each into another app and confirm only the chosen/mixed audio is present while memory stays broadly flat.
- Upload a large original clip and a selected-audio variant; interrupt and retry a resumable upload, then confirm the remote file plays and the local file/temp cache is preserved or cleaned according to settings.
