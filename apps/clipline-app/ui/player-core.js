// Pure review-player logic: formatting, trim clamping, timeline math, marker
// navigation, keyboard intents. No DOM, no Tauri — tests/player_core.rs
// evaluates this file in Boa, so it must stay dependency-free.
const PlayerCore = (() => {
  const MIN_TRIM_GAP_S = 0.1;
  const MARKER_EPSILON_S = 0.05;

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
      case "Escape":
        return { kind: "close" };
      default:
        return null;
    }
  };

  return {
    MIN_TRIM_GAP_S,
    MARKER_EPSILON_S,
    fmtBytes,
    fmtDur,
    fmtTenths,
    fmtAgo,
    clampTime,
    percentFor,
    timelineTime,
    resolveTrim,
    trimDrag,
    trimSummary,
    nextMarker,
    prevMarker,
    markerSummary,
    keyIntent,
  };
})();

globalThis.PlayerCore = PlayerCore;
