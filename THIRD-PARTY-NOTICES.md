# Third-Party Notices

Clipline's first-party code is permissively licensed (see the repository
license). It additionally relies on the components below; their licenses and
attribution requirements are reproduced here.

## FFmpeg (LGPL v2.1+)

Clipline's optional encoder tier (HEVC / AV1 / additional H.264 backends)
works by invoking a bundled **FFmpeg** executable as a separate process. It
pipes raw NV12 frames to `ffmpeg.exe` and reads back the encoded elementary
stream; Clipline does not link the FFmpeg libraries into its own binary.

- Clipline ships an **LGPL** build of FFmpeg (configured **without**
  `--enable-gpl` / `--enable-nonfree`). It contains SVT-AV1 (BSD) and the
  GPU vendor encoders; it does **not** contain GPL components such as
  `libx264` or `libx265`.
- FFmpeg is licensed under the GNU Lesser General Public License (LGPL)
  version 2.1 or later. A copy of the LGPL and the GPL is distributed
  alongside the FFmpeg binaries in the same folder.
- **Source code:** the exact, unmodified FFmpeg source corresponding to the
  shipped build is available from the build provider
  (<https://github.com/BtbN/FFmpeg-Builds>) and from the FFmpeg project
  (<https://ffmpeg.org/download.html>). Clipline applies no patches to
  FFmpeg.
- The FFmpeg libraries are dynamically loaded (separate `.dll`s) and the
  executable is independently replaceable, satisfying LGPL §6's requirement
  that users can substitute a modified version of the library.

> This software uses code of <a href="https://ffmpeg.org">FFmpeg</a> licensed
> under the <a href="https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html">LGPLv2.1</a>.

### SVT-AV1 (BSD-3-Clause + AOM patent terms)

The software AV1 encoder is the Scalable Video Technology for AV1 (SVT-AV1),
distributed inside the LGPL FFmpeg build under the BSD-3-Clause license with
the Alliance for Open Media Patent License 1.0.

## Codec patents

H.264 and HEVC carry patent-pool obligations that are independent of
FFmpeg's software license. Relying on GPU/OS-provided encoders typically
conveys the relevant patent license to the user; confirm obligations before
redistributing encoded output or the encoders themselves. AV1/Opus are
royalty-free by design (see `ddoc.md` §4 for the Sisvel/Dolby AV1 caveat).
