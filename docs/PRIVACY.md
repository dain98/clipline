# Privacy and diagnostic data

Clipline keeps detailed, bounded diagnostics on the local computer. This is not
telemetry: no log, crash, recording, or usage event is uploaded automatically.
Clipline Cloud is self-hosted and has no diagnostic intake API.

## Local diagnostics

The app writes structured JSONL logs under `%APPDATA%\Clipline\logs`, falling
back to a per-user temporary directory if AppData is unavailable. Settings >
Support displays the actual directory, and the tray menu can open it even when
the main WebView is unusable.

Logs retain five 4 MiB generations for at most seven days. The in-memory queue
is lossy so recording threads never wait for disk. Panic details and forced
backtraces use separately bounded files. Clipline does not create minidumps or
capture recordings, frames, or screenshots for diagnostics.

## Private reports

A report requires three deliberate steps: enter a description, prepare and
review the file list/size, then confirm **Send Private Report**. The description
is transmitted exactly as entered. A failed submission may be retried or saved
locally; it is never retried in the background.

The package is limited to sanitized logs, a manifest, non-identifying system
capabilities/counts, selected safe settings, and a runtime health snapshot. It
never includes recordings, clips, media filenames, screenshots, directory
listings, raw `settings.json`, account identities, device IDs, credentials, or
authorization/request bodies. Logging avoids those values at the source and
the exporter applies a second redaction pass with bundle-local aliases.

Reports go only to the compile-time official HTTPS `clipline-support` endpoint.
They are anonymous, private to the single numeric GitHub-allowlisted
administrator, have no public lookup page, and are deleted after 30 days.
Clipline Cloud—including any self-hosted instance—never receives them.
