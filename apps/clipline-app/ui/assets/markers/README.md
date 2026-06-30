# Timeline marker icons

`kill.png`, `death.png`, `assist.png`, `dragon.png`, `baron.png`, and
`turret.png` are first-party silhouette icons created or provided for
Clipline's review-timeline markers. They were not extracted, traced, or copied
from any game's assets, and no third-party copyright is claimed over them.

As first-party project assets they are covered by Clipline's own license
(**MIT OR Apache-2.0**); they are not third-party material, so they are not
listed in `THIRD-PARTY-NOTICES.md`.

They are rendered as tinted CSS masks (only the alpha channel is used) in the
review timeline — see `apps/clipline-app/ui/main.js` (`MARKER_IMAGES`) and
`apps/clipline-app/ui/styles.css` (`.marker .glyph.img`).

Keep each marker PNG on a 320x320 transparent canvas with a 280px-tall visible
alpha box, centered vertically. The timeline and event rail intentionally rely
on the image assets having matching alpha bounds instead of per-kind CSS scale
overrides.
