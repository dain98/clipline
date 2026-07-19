# M-23 Shared D3D11 Multithread Boundary Plan

> **Finding:** M-23 — public shared-device constructors do not enforce D3D multithread protection.

## Goal

Make every safe capture/conversion/encoder entry point that accepts a caller-provided D3D11 device
establish the same immediate-context multithread protection as Clipline-created devices before any
context, frame-pool, duplication, readback, or encoder work begins.

## TDD sequence

- [ ] Add a WARP-backed test that creates a deliberately unprotected D3D11 device, verifies the
  centralized guard enables protection, and verifies a public readback boundary does the same.
- [ ] Extend the real caller-provided WGC constructor test to disable protection before construction
  and assert construction restores it when WGC is available.
- [ ] Run the focused tests and record the expected red compile/behavior failure.
- [ ] Centralize an idempotent `ensure_multithread_protected` helper in the Windows D3D11 wrapper and
  use it for Clipline-created devices.
- [ ] Invoke the guard in WGC and DXGI capture construction, GPU/CPU FFmpeg encoder construction,
  D3D video conversion/readback, and Media Foundation encoder construction.
- [ ] Run focused capture tests, fresh-cache capture Clippy, CI-mode workspace tests, and workspace
  Clippy with warnings denied.
- [ ] Rebuild and open Clipline for a native capture/library smoke check.
- [ ] Update `handoff.md`, the combined audit ledger, and the final manual acceptance checklist only
  where real concurrent capture hardware remains necessary.

## Invariants

- [ ] A caller-provided device is protected before Clipline obtains or shares its immediate context.
- [ ] The guard is idempotent and verifies protection remains enabled after requesting it.
- [ ] Failure to query or establish protection is returned through the existing safe constructor
  error type; no constructor proceeds with an undocumented unsafe precondition.
- [ ] Clipline-created hardware and WARP devices retain their current protected behavior.
- [ ] Protection applies equally to WGC callbacks, DXGI duplication, video-processor conversion,
  CPU/GPU readback, FFmpeg device retention, and D3D-aware MFT use.

## Commits

- `docs(plan): define M-23 D3D sharing remediation`
- `fix(capture): enforce D3D multithread protection`
- `docs(audit): close shared D3D device finding`
