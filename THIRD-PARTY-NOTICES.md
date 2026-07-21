# Third-Party Notices

Clipline's first-party code is permissively licensed (see the repository
license). It additionally relies on the components below; their licenses and
attribution requirements are reproduced here.

## FFmpeg (LGPL v3+, selected build)

Clipline invokes a bundled **FFmpeg** executable as a separate process for
gallery poster extraction and for its optional encoder tier (HEVC / AV1 /
additional H.264 backends). For encoding, it pipes raw NV12 frames to
`ffmpeg.exe` and reads back the encoded elementary stream; Clipline does not
link the FFmpeg libraries into its own binary.

- Clipline pins BtbN's retained `autobuild-2026-06-30-13-34` x64 shared
  archive (`ffmpeg n8.1.2-21-gce3c09c101-20260630`). It enables
  `--enable-version3` and is configured **without** `--enable-gpl` or
  `--enable-nonfree`. It contains SVT-AV1 (BSD) and GPU vendor encoders, but
  not GPL components such as `libx264` or `libx265`.
- FFmpeg is generally available under the GNU Lesser General Public License
  version 2.1 or later; the selected version3 build is distributed under
  LGPL v3 or later. Its `LICENSE.txt` is bundled beside the executable.
- **Exact source and build provenance:** the corresponding FFmpeg revision is
  <https://github.com/FFmpeg/FFmpeg/commit/ce3c09c101>, and the retained build
  release is
  <https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-06-30-13-34>.
  Clipline applies no FFmpeg patches. `PROVENANCE.json` beside the binaries
  records the archive URL/hash, version/configuration, source links, and the
  size and SHA-256 of every shipped runtime file.
- The FFmpeg libraries are dynamically loaded (separate `.dll`s) and the
  executable is independently replaceable, satisfying LGPL §6's requirement
  that users can substitute a modified version of the library.

> This software uses code of <a href="https://ffmpeg.org">FFmpeg</a> licensed
> under the <a href="https://www.gnu.org/licenses/lgpl-3.0.html">LGPLv3</a>.

### SVT-AV1 (BSD-3-Clause + AOM patent terms)

The software AV1 encoder is the Scalable Video Technology for AV1 (SVT-AV1),
distributed inside the LGPL FFmpeg build under the BSD-3-Clause license with
the Alliance for Open Media Patent License 1.0.

## League of Legends event rail assets

The bundled first-party League of Legends presentation seed includes small
right-side event rail objective icons and minion actor portraits mirrored by
CommunityDragon from League client match-history assets. They are used only for
League event rail presentation; the timeline marker icons are first-party
generated assets.

- **Source:** <https://raw.communitydragon.org/latest/plugins/rcp-fe-lol-match-history/global/default/>
- Riot Games owns the underlying League of Legends artwork and trademarks.
- CommunityDragon provides a public mirror of League client assets; Clipline
  does not claim third-party rights over these images.

## Codec patents

H.264 and HEVC carry patent-pool obligations that are independent of
FFmpeg's software license. Relying on GPU/OS-provided encoders typically
conveys the relevant patent license to the user; confirm obligations before
redistributing encoded output or the encoders themselves. AV1/Opus are
royalty-free by design (see `ddoc.md` §4 for the Sisvel/Dolby AV1 caveat).
