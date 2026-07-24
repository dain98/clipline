# Private Bug Reports and Structured Diagnostics Plan

**Goal:** Give Clipline an always-on, bounded, privacy-safe diagnostic trail and an explicit
in-app workflow for sending anonymous bug reports to a separate administrator-only intake
service.

## Desktop logging and capture

- [ ] Replace the ad hoc diagnostic sink and production `eprintln!` calls with structured JSONL
      tracing on a bounded, lossy, off-thread writer.
- [ ] Retain five 4 MiB generations for at most seven days, preserve legacy logs during migration,
      expose dropped-line health, and support a flush/snapshot barrier for report preparation.
- [ ] Install an early bounded panic/backtrace hook and bridge bounded, rate-limited frontend errors
      and unhandled promise rejections into the same diagnostic stream.
- [ ] Instrument startup, settings, capture, replay, library, cloud, game, updater, tray, and
      recoverable background boundaries without logging per-frame data or secrets.

## Private support bundle and UI

- [ ] Build a redacted ZIP containing only bounded logs, panic records, a manifest, sanitized
      system/runtime information, and a safe settings projection.
- [ ] Add expiring prepared-report state and Tauri commands for prepare, confirm/upload,
      cancel/discard, save locally, diagnostics-folder access, and frontend diagnostics.
- [ ] Add a Support settings tab with description validation, an exact disclosure and bundle
      preview, explicit confirmation, progress/retry/save behavior, and the returned report ID.
- [ ] Add a native tray action that reveals the active diagnostics directory when the webview is
      unavailable.

## Dedicated intake service

- [ ] Create a separate `clipline-support` Rust repository using Axum, SQLite/WAL, and private
      S3-compatible storage.
- [ ] Implement bounded anonymous multipart ingestion, schema/archive/hash validation,
      idempotency, HMAC-based rate limiting without stored IPs, global quotas, and 30-day deletion.
- [ ] Implement a GitHub-OAuth administrator allowlist and a server-rendered private inbox for
      report status, notes, attachment download, and deletion.
- [ ] Provide a non-root container, migrations, health/readiness endpoints, cleanup/backup jobs,
      deployment configuration, and an integration path using MinIO.

## Verification and rollout

- [ ] Cover rotation, snapshots, queue pressure, panic/frontend capture, redaction, bundle bounds,
      staging lifetime, upload safety, UI contracts, and endpoint failure modes with desktop tests.
- [ ] Cover ingestion attacks, limits, idempotency, storage/database failures, OAuth/session/CSRF
      boundaries, private downloads, and retention with service tests.
- [ ] Run desktop workspace tests and warning-denied Clippy, service tests and warning-denied
      Clippy, then exercise an end-to-end fixture against local object storage.
- [ ] Update the design, privacy/troubleshooting, deployment, and handoff documentation; rebuild
      and open Clipline for manual acceptance.

## Fixed product boundaries

- [ ] Clipline Cloud remains untouched and self-hosted; the official support endpoint is injected
      at release build time and is never user-configurable.
- [ ] Reports are anonymous, explicitly submitted, visible only to the one allowlisted
      administrator, and fully deleted after 30 days.
- [ ] No recording, screenshot, raw settings, credential, account identity, automatic telemetry,
      Windows minidump, public issue, or delayed automatic upload enters the workflow.
