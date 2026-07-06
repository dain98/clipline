Release builds stage the LGPL FFmpeg runtime in this directory before running
`cargo tauri build`.

The actual binaries are ignored so they do not land in git. Use the release
prep script or copy the known LGPL shared build here, preserving:

- `ffmpeg.exe`
- FFmpeg DLL dependencies
- LGPL/GPL license texts from the distribution
