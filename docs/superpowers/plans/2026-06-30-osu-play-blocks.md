# osu! Play Blocks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add osu! play blocks for full-session recordings using post-session API enrichment and a Cloud pure-proxy design.

**Architecture:** The desktop stores normalized osu! plays in clip marker sidecars and renders them as timeline intervals plus a Set plays rail. Clipline Cloud owns osu! API credentials and proxying; the desktop never stores osu! secrets.

**Tech Stack:** Rust workspace, Tauri commands, vanilla HTML/CSS/JS, Boa-tested `player-core.js`, Clipline Cloud API contract.

---

## Summary

Build osu! play blocks with a **Cloud pure-proxy** design: the desktop uses its existing Clipline Cloud device token; Cloud calls osu! with server-held credentials; the desktop never receives osu! tokens or secrets.

Before broker work, run a real API spike to verify whether `client_credentials + public + include_fails=1` can fetch recent failed plays. If yes, no osu! user OAuth storage is needed. If no, fall back to authorization-code login handled entirely by Cloud.

## Tasks

- [ ] Add a real API spike script and fixture path for recent osu! scores.
- [ ] Extend `ClipMarkers` with backward-compatible `plays: Vec<ClipPlay>`.
- [ ] Update sidecar crop/export/content/delete/quota behavior for play sidecars and pending enrichment files.
- [ ] Add an `osu` supported-game manifest and `GameId::Osu`.
- [ ] Add osu!-specific supported-game settings UI with Cloud-required account copy.
- [ ] Add timeline play interval helpers, rendering layer, Set plays rail, and gallery summary.
- [ ] Add desktop enrichment job/pending-record scaffolding that calls a Cloud pure-proxy endpoint.
- [ ] Validate with targeted tests, workspace tests, clippy, JS syntax checks, and a manual app launch.

## Key Decisions

- The desktop calls Clipline Cloud only; Cloud calls osu!.
- Use app-level `client_credentials` if the spike proves `include_fails=1` is visible through public scope.
- Use authorization-code user auth only if the spike proves failed plays require it.
- Require `ended_at`; prefer `started_at`; derive start from beatmap length when needed and mark it estimated.
- Use UTC windows plus skew tolerance.
- Keep failed submitted plays; unsubmitted retries cannot appear.
- Use a 500-score pagination ceiling and surface a user-visible partial-results status when hit.
