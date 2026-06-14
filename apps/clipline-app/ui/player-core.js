// Pure review-player logic: formatting, trim clamping, timeline math, marker
// navigation, keyboard intents. No DOM, no Tauri — tests/player_core.rs
// evaluates this file in Boa, so it must stay dependency-free.
const PlayerCore = (() => {
  const MIN_TRIM_GAP_S = 0.1;
  const MARKER_EPSILON_S = 0.05;
  const OVERLAY_HIDE_MS = 2000;

  // YouTube grammar: controls pin while paused, fade when playing and idle.
  const overlayVisible = (paused, idleMs) => paused || idleMs < OVERLAY_HIDE_MS;

  const fmtBytes = (bytes) => {
    const mb = bytes / (1024 * 1024);
    if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
    return `${mb.toFixed(1)} MB`;
  };

  // Round the total before splitting so 59.6 carries to "1:00", not "0:60".
  const fmtDur = (s) => {
    if (s == null || !Number.isFinite(s)) return "?";
    const total = Math.round(s);
    const minutes = Math.floor(total / 60);
    const seconds = total - minutes * 60;
    return `${minutes}:${String(seconds).padStart(2, "0")}`;
  };

  // m:ss.t — the readout/trim precision. Same carry rule, in tenths.
  const fmtTenths = (s) => {
    if (s == null || !Number.isFinite(s)) return "?";
    const tenths = Math.round(s * 10);
    const minutes = Math.floor(tenths / 600);
    const rest = tenths - minutes * 600;
    const seconds = Math.floor(rest / 10);
    return `${minutes}:${String(seconds).padStart(2, "0")}.${rest % 10}`;
  };

  const fmtAgo = (nowUnixS, thenUnixS) => {
    const d = Math.max(0, nowUnixS - thenUnixS);
    if (d < 90) return `${Math.round(d)}s ago`;
    if (d < 5400) return `${Math.round(d / 60)}m ago`;
    return `${Math.round(d / 3600)}h ago`;
  };

  const settingDurationLabel = (seconds) => {
    const total = Math.max(0, Math.round(seconds || 0));
    if (total < 60) return `${total} sec`;
    const minutes = Math.floor(total / 60);
    const rest = total - minutes * 60;
    const minutePart = `${minutes} min`;
    return rest ? `${minutePart} ${rest} sec` : minutePart;
  };

  const QUALITY_PRESETS = [
    { id: "compact", label: "Compact", hint: "smaller files" },
    { id: "balanced", label: "Balanced", hint: "good default" },
    { id: "sharp", label: "Sharp", hint: "more detail" },
    { id: "maximum", label: "Maximum", hint: "largest files" },
  ];

  const QUALITY_BITRATES_MBPS = {
    source: [6, 12, 24, 40],
    "1440p": [6, 12, 24, 40],
    "1080p": [4, 8, 16, 24],
    "720p": [2.5, 5, 8, 12],
    "480p": [1.5, 3, 5, 8],
  };

  const SMOOTHNESS_PRESETS = [
    { fps: 30, label: "30 FPS", hint: "lighter on the PC" },
    { fps: 60, label: "60 FPS", hint: "good default for most games" },
    { fps: 90, label: "90 FPS", hint: "smoother for high-refresh play" },
    { fps: 120, label: "120 FPS", hint: "best for high-refresh footage" },
  ];

  const OUTPUT_RESOLUTION_OPTIONS = [
    { id: "source", label: "Source", hint: "uses the captured size" },
    { id: "1440p", label: "1440p", hint: "up to 2560 x 1440" },
    { id: "1080p", label: "1080p", hint: "up to 1920 x 1080" },
    { id: "720p", label: "720p", hint: "up to 1280 x 720" },
    { id: "480p", label: "480p", hint: "up to 854 x 480" },
  ];

  const nearestPresetIndex = (presets, value, field) => {
    let best = 0;
    let bestDistance = Infinity;
    for (let i = 0; i < presets.length; i++) {
      const distance = Math.abs(presets[i][field] - value);
      if (distance < bestDistance) {
        best = i;
        bestDistance = distance;
      }
    }
    return best;
  };

  const qualityBitrates = (outputResolution) =>
    QUALITY_BITRATES_MBPS[outputResolutionOption(outputResolution).id] || QUALITY_BITRATES_MBPS.source;

  const recordingQualityPreset = (index, outputResolution = "source") => {
    const clamped = Math.max(0, Math.min(QUALITY_PRESETS.length - 1, Math.round(index || 0)));
    return { ...QUALITY_PRESETS[clamped], bitrate: qualityBitrates(outputResolution)[clamped] };
  };

  const qualityIndexForBitrate = (mbps, outputResolution = "source") => {
    const bitrates = qualityBitrates(outputResolution);
    const presets = QUALITY_PRESETS.map((preset, index) => ({ ...preset, bitrate: bitrates[index] }));
    return nearestPresetIndex(presets, Number(mbps) || bitrates[1], "bitrate");
  };

  const qualityIndexForId = (id) => {
    const index = QUALITY_PRESETS.findIndex((preset) => preset.id === id);
    return index >= 0 ? index : 1;
  };

  const smoothnessPreset = (index) =>
    SMOOTHNESS_PRESETS[Math.max(0, Math.min(SMOOTHNESS_PRESETS.length - 1, Math.round(index || 0)))];

  const smoothnessIndexForFps = (fps) =>
    nearestPresetIndex(SMOOTHNESS_PRESETS, Number(fps) || SMOOTHNESS_PRESETS[1].fps, "fps");

  const outputResolutionOption = (id) =>
    OUTPUT_RESOLUTION_OPTIONS.find((option) => option.id === id) || OUTPUT_RESOLUTION_OPTIONS[0];

  const captureSourceLabel = (settings) => {
    switch (settings && settings.capture_mode) {
      case "window_title":
        return settings.window_title ? `Window: ${settings.window_title}` : "Window";
      case "display_region":
        return "Display region";
      default:
        return "Desktop";
    }
  };

  const captureStatusLabel = (source, recording, fullSession) => {
    if (!recording) return "Recording stopped";
    return `${fullSession ? "Recording" : "Capturing"} ${source}`;
  };

  const clampTime = (value, duration) => {
    // Unknown duration (metadata not loaded yet) must not clamp seeks to zero.
    const max = duration > 0 ? duration : Number.MAX_SAFE_INTEGER;
    return Math.max(0, Math.min(max, value));
  };

  const percentFor = (time, duration) => {
    if (!duration) return 0;
    return Math.max(0, Math.min(100, (time / duration) * 100));
  };

  const timelineTime = (clientX, rectLeft, rectWidth, duration) => {
    const x = Math.max(0, Math.min(rectWidth, clientX - rectLeft));
    return clampTime((x / rectWidth) * duration, duration);
  };

  const resolveTrim = (start, end, duration) => {
    let nextStart = clampTime(Number.isFinite(start) ? start : 0, duration);
    let nextEnd = clampTime(
      Number.isFinite(end) && end > 0 ? end : duration,
      duration
    );
    if (duration && nextEnd <= nextStart) {
      nextEnd = Math.min(duration, nextStart + MIN_TRIM_GAP_S);
    }
    if (nextEnd <= nextStart) {
      nextStart = Math.max(0, nextEnd - MIN_TRIM_GAP_S);
    }
    return { start: nextStart, end: nextEnd };
  };

  const trimDrag = (kind, time, start, end, duration) => {
    if (kind === "in") {
      return resolveTrim(Math.min(time, end - MIN_TRIM_GAP_S), end, duration);
    }
    return resolveTrim(start, Math.max(time, start + MIN_TRIM_GAP_S), duration);
  };

  const trimSummary = (start, end) =>
    `keeps ${fmtTenths(start)} – ${fmtTenths(end)} · ${Math.max(0, end - start).toFixed(1)} s`;

  const nextMarker = (markers, currentS) => {
    if (!markers.length) return null;
    for (const m of markers) {
      if (m.t_s > currentS + MARKER_EPSILON_S) return m;
    }
    return markers[0];
  };

  const prevMarker = (markers, currentS) => {
    if (!markers.length) return null;
    for (let i = markers.length - 1; i >= 0; i--) {
      if (markers[i].t_s < currentS - MARKER_EPSILON_S) return markers[i];
    }
    return markers[markers.length - 1];
  };

  const markerSummary = (markers) => {
    if (!markers.length) return "no markers";
    return markers.length === 1 ? "1 marker" : `${markers.length} markers`;
  };

  // EventKind variant name -> visual category. Unknown kinds degrade to info.
  const MARKER_CATEGORIES = {
    ChampionKill: "kill",
    FirstBlood: "kill",
    Multikill: "spree",
    Ace: "spree",
    DragonKill: "objective",
    HeraldKill: "objective",
    BaronKill: "objective",
    TurretKilled: "structure",
    InhibKilled: "structure",
    FirstBrick: "structure",
  };
  const MARKER_GLYPHS = {
    kill: "✕",
    spree: "★",
    objective: "◆",
    structure: "▣",
    info: "•",
  };

  const markerStyle = (kind) => {
    const cls = MARKER_CATEGORIES[kind] || "info";
    return { glyph: MARKER_GLYPHS[cls], cls };
  };

  const DIGEST_NOUNS = {
    kill: ["kill", "kills"],
    spree: ["spree", "sprees"],
    objective: ["objective", "objectives"],
    structure: ["structure", "structures"],
    info: ["event", "events"],
  };

  const markerDigest = (markers) => {
    const counts = {};
    for (const m of markers) {
      const cls = MARKER_CATEGORIES[m.kind] || "info";
      counts[cls] = (counts[cls] || 0) + 1;
    }
    return Object.keys(DIGEST_NOUNS)
      .filter((cls) => counts[cls])
      .map((cls) => `${counts[cls]} ${DIGEST_NOUNS[cls][counts[cls] === 1 ? 0 : 1]}`)
      .join(" · ");
  };

  const RULER_STEPS_S = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600];

  const rulerMarks = (duration, maxMarks) => {
    if (!Number.isFinite(duration) || duration <= 0) return [];
    const step =
      RULER_STEPS_S.find((s) => duration / s <= maxMarks - 1) ??
      RULER_STEPS_S[RULER_STEPS_S.length - 1];
    const marks = [];
    for (let t = 0; t <= duration; t += step) {
      marks.push({ t, label: fmtDur(t) });
    }
    return marks;
  };

  // Bucket clips by session folder; legacy root clips fall under "Earlier".
  // Groups and the clips inside them sort newest-first.
  const sessionGroups = (clips) => {
    const order = [];
    const byLabel = {};
    for (const c of clips) {
      const label = c.session ? c.session : "Earlier";
      if (!byLabel[label]) {
        byLabel[label] = [];
        order.push(label);
      }
      byLabel[label].push(c);
    }
    const groups = order.map((label) => ({
      label,
      clips: byLabel[label].slice().sort((a, b) => b.modified_unix - a.modified_unix),
    }));
    groups.sort((a, b) => b.clips[0].modified_unix - a.clips[0].modified_unix);
    return groups;
  };

  const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

  const formatClipTitle = (month0, day, hours, minutes) => {
    const h12 = hours % 12 === 0 ? 12 : hours % 12;
    const ampm = hours < 12 ? "AM" : "PM";
    return `${MONTHS[month0]} ${day} · ${h12}:${String(minutes).padStart(2, "0")} ${ampm}`;
  };

  // Classify a library clip by its on-disk name so each kind can carry its own
  // icon. Trimmed exports always include `_trim_`; full-session captures are
  // written with a `session_` prefix.
  const clipKind = (name) => {
    const n = name || "";
    if (/_trim_/.test(n)) return "trim";
    if (/^session_/.test(n)) return "session";
    return "replay";
  };

  const keyIntent = (code, shiftKey) => {
    switch (code) {
      case "Space":
      case "KeyK":
        return { kind: "toggle-play" };
      case "ArrowLeft":
      case "KeyJ":
        return { kind: "seek-by", seconds: shiftKey ? -1 : -5 };
      case "ArrowRight":
      case "KeyL":
        return { kind: "seek-by", seconds: shiftKey ? 1 : 5 };
      case "Comma":
        return { kind: "seek-by", seconds: -0.1 };
      case "Period":
        return { kind: "seek-by", seconds: 0.1 };
      case "KeyI":
        return { kind: "set-in" };
      case "KeyO":
        return { kind: "set-out" };
      case "KeyM":
        return { kind: shiftKey ? "prev-marker" : "next-marker" };
      case "KeyF":
        return { kind: "toggle-focus" };
      case "Escape":
        return { kind: "close" };
      default:
        return null;
    }
  };

  const functionKeyNumber = (ev) => {
    const raw = String(ev.code || ev.key || "").toUpperCase();
    const match = raw.match(/^F([1-9]|1[0-9]|2[0-4])$/);
    return match ? Number(match[1]) : null;
  };

  const hotkeyFromKeyEvent = (ev) => {
    if (ev.code === "Escape" || ev.key === "Escape") return { kind: "cancel" };
    if (
      ev.code === "ControlLeft" ||
      ev.code === "ControlRight" ||
      ev.code === "AltLeft" ||
      ev.code === "AltRight" ||
      ev.code === "ShiftLeft" ||
      ev.code === "ShiftRight"
    ) {
      return { kind: "pending", message: "Now press an F-key." };
    }

    const key = functionKeyNumber(ev);
    if (!key) {
      return { kind: "invalid", message: "Use F1-F11 or F13-F24 as the shortcut key." };
    }
    if (key === 12) {
      return { kind: "invalid", message: "F12 is reserved by Windows for debuggers." };
    }

    const parts = [];
    if (ev.ctrlKey) parts.push("Ctrl");
    if (ev.altKey) parts.push("Alt");
    if (ev.shiftKey) parts.push("Shift");
    parts.push(`F${key}`);
    return { kind: "captured", value: parts.join("+") };
  };

  const displayBounds = (displays) => {
    if (!displays.length) return { x: 0, y: 0, width: 0, height: 0 };
    const left = Math.min(...displays.map((d) => d.x));
    const top = Math.min(...displays.map((d) => d.y));
    const right = Math.max(...displays.map((d) => d.x + d.width));
    const bottom = Math.max(...displays.map((d) => d.y + d.height));
    return { x: left, y: top, width: right - left, height: bottom - top };
  };

  const displayMapLayout = (displays, viewportW, viewportH, padding = 10) => {
    const bounds = displayBounds(displays);
    const innerW = Math.max(1, viewportW - padding * 2);
    const innerH = Math.max(1, viewportH - padding * 2);
    const scale = bounds.width && bounds.height
      ? Math.min(innerW / bounds.width, innerH / bounds.height)
      : 1;
    return {
      bounds,
      scale,
      width: bounds.width * scale + padding * 2,
      height: bounds.height * scale + padding * 2,
      displays: displays.map((d) => ({
        id: d.id,
        left: padding + (d.x - bounds.x) * scale,
        top: padding + (d.y - bounds.y) * scale,
        width: d.width * scale,
        height: d.height * scale,
      })),
    };
  };

  const displayMapHeight = (displays, viewportW, padding = 10, minH = 180, maxH = 420) => {
    const bounds = displayBounds(displays);
    if (!bounds.width || !bounds.height) return minH;
    const innerW = Math.max(1, viewportW - padding * 2);
    const height = bounds.height * (innerW / bounds.width) + padding * 2;
    return Math.max(minH, Math.min(maxH, height));
  };

  const regionForDisplay = (display) => ({
    display_id: display.id,
    x: display.x,
    y: display.y,
    width: display.width,
    height: display.height,
  });

  const evenSize = (value, max) => {
    const clamped = Math.max(2, Math.min(Math.round(value || 2), max));
    return clamped % 2 === 0 ? clamped : Math.max(2, clamped - 1);
  };

  const clampRegionToDisplay = (region, display) => {
    const width = evenSize(region.width, display.width);
    const height = evenSize(region.height, display.height);
    const minX = display.x;
    const minY = display.y;
    const maxX = display.x + display.width - width;
    const maxY = display.y + display.height - height;
    const x = Math.max(minX, Math.min(maxX, Math.round(region.x || 0)));
    const y = Math.max(minY, Math.min(maxY, Math.round(region.y || 0)));
    return { display_id: display.id, x, y, width, height };
  };

  const alignRegion = (region, display, align) => {
    const next = clampRegionToDisplay(region, display);
    switch (align) {
      case "left":
        next.x = display.x;
        break;
      case "right":
        next.x = display.x + display.width - next.width;
        break;
      case "top":
        next.y = display.y;
        break;
      case "bottom":
        next.y = display.y + display.height - next.height;
        break;
      case "center":
        next.x = Math.round(display.x + (display.width - next.width) / 2);
        next.y = Math.round(display.y + (display.height - next.height) / 2);
        break;
    }
    return next;
  };

  // --- Encoder codec playback support (Settings) ---
  // Codecs the in-app review player may need an OS extension to decode.
  // H.264 always plays in WebView2; HEVC/AV1 are probed at runtime via
  // canPlayType. The mime strings name concrete profiles the muxer emits.
  const VIDEO_DECODE_PROBES = [
    { codec: "hevc", mime: 'video/mp4; codecs="hvc1.1.6.L93.B0"' },
    { codec: "av1", mime: 'video/mp4; codecs="av01.0.04M.08"' },
  ];

  const videoDecodeProbes = () => VIDEO_DECODE_PROBES.map((p) => ({ ...p }));

  // One-line caveat shown under the encoder dropdown when the selected
  // encoder's codec is not in the set the player can decode; null otherwise.
  const encoderCodecCaveat = (codecKey, decodableCodecs) => {
    if (!codecKey || codecKey === "h264") return null;
    if ((decodableCodecs || []).includes(codecKey)) return null;
    const name =
      codecKey === "hevc" ? "HEVC" : codecKey === "av1" ? "AV1" : String(codecKey).toUpperCase();
    return `${name} may not play in the in-app review player on this PC. The clip still records and opens in other players.`;
  };

  return {
    MIN_TRIM_GAP_S,
    MARKER_EPSILON_S,
    OVERLAY_HIDE_MS,
    overlayVisible,
    videoDecodeProbes,
    encoderCodecCaveat,
    fmtBytes,
    fmtDur,
    fmtTenths,
    fmtAgo,
    settingDurationLabel,
    recordingQualityPreset,
    qualityIndexForBitrate,
    qualityIndexForId,
    smoothnessPreset,
    smoothnessIndexForFps,
    outputResolutionOption,
    captureSourceLabel,
    captureStatusLabel,
    clampTime,
    percentFor,
    timelineTime,
    resolveTrim,
    trimDrag,
    trimSummary,
    nextMarker,
    prevMarker,
    markerSummary,
    markerStyle,
    markerDigest,
    rulerMarks,
    sessionGroups,
    formatClipTitle,
    clipKind,
    keyIntent,
    hotkeyFromKeyEvent,
    displayBounds,
    displayMapLayout,
    displayMapHeight,
    regionForDisplay,
    clampRegionToDisplay,
    alignRegion,
  };
})();

globalThis.PlayerCore = PlayerCore;
