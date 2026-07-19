# Single-PUT Upload Content Type Fix

**Goal:** Make Clipline Cloud single-PUT uploads satisfy the server's MP4 media-type contract.

## Root cause

The chunked proxy upload path sends `Content-Type: video/mp4`, but the single-PUT path sends only
`Content-Length`. The Cloud server now rejects that request with HTTP 400 because single-PUT
uploads require an explicit MP4 content type. The existing single-PUT regression test verifies the
body but does not verify the header.

## Implementation

- [ ] Extend the single-PUT upload test to require `Content-Type: video/mp4` and confirm it fails
      against the current implementation.
- [ ] Add the missing header to the streamed single-PUT request without changing authentication,
      body streaming, progress, or timeout behavior.
- [ ] Run the focused upload test, workspace tests, and warning-denied workspace Clippy.
- [ ] Rebuild and reopen Clipline for a real-account upload retest.

## Manual retest

1. Upload a small local MP4 through a deployment that selects `single_put` mode.
2. Confirm the upload no longer fails with the missing `Content-Type` HTTP 400 response.
3. Confirm byte progress advances and the Cloud card reaches processing/ready state.
4. Confirm the original local clip remains available unless the configured verified-delete policy
   explicitly permits removal.
