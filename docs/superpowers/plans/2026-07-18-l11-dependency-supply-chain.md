# L-11 Dependency and CI Supply-Chain Plan

> **Finding:** L-11 — a vulnerable locked dependency, missing advisory gate, and mutable CI action
> revisions leave avoidable supply-chain drift.

## Goal

Remove the known `anyhow` advisory, make vulnerability scanning a required reproducible workflow,
pin every remote GitHub Action to a reviewed commit, and automate routine update proposals without
silently accepting advisory exceptions.

## TDD sequence

- [ ] Add a repository-security contract that rejects non-SHA remote action revisions, requires a
  RustSec workflow and explicit empty ignore policy, requires Cargo/GitHub-Actions Dependabot entries,
  and verifies the locked `anyhow` version is at least 1.0.103; run it red.
- [ ] Update only `anyhow` in `Cargo.lock` to the fixed release selected by Cargo.
- [ ] Pin checkout, Rust toolchain, Rust cache, and RustSec audit actions to full upstream commit SHAs
  with human-readable version/channel comments and least-privilege workflow permissions.
- [ ] Add a dependency-security workflow for dependency changes, manual runs, and a weekly advisory
  refresh, with RustSec output/failure preserved for fork pull requests.
- [ ] Add `.cargo/audit.toml` with no ignores and a documented owner/rationale/expiry/removal process
  for any future exception.
- [ ] Add weekly Dependabot proposals for Cargo dependencies and GitHub Actions revisions.
- [ ] Run the focused repository contract, validate workflow/config syntax, run a local RustSec audit,
  then run CI-mode workspace tests and workspace Clippy with warnings denied.
- [ ] Update `handoff.md` and the combined audit ledger; no native-app smoke is required for CI-only
  metadata and a lockfile patch with unchanged application behavior.

## Invariants

- [ ] `Cargo.lock` cannot select an `anyhow` release affected by RUSTSEC-2026-0190.
- [ ] Every remote `uses:` reference in repository workflows is a 40-character commit SHA.
- [ ] Version/channel comments keep immutable revisions reviewable.
- [ ] Vulnerability scans fail on actionable advisories and run even when the lockfile is unchanged.
- [ ] Advisory ignores default to empty and require an owner, reachability rationale, and expiry.
- [ ] Cargo and action pin updates arrive as reviewable automated pull requests.
- [ ] CI tokens have only the read/check/issue permissions required by their jobs.

## Commits

- `docs(plan): define L-11 supply-chain gates`
- `fix(ci): pin and audit dependencies`
- `docs(audit): close dependency supply-chain finding`
