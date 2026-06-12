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

  const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

  const formatClipTitle = (month0, day, hours, minutes) => {
    const h12 = hours % 12 === 0 ? 12 : hours % 12;
    const ampm = hours < 12 ? "AM" : "PM";
    return `${MONTHS[month0]} ${day} · ${h12}:${String(minutes).padStart(2, "0")} ${ampm}`;
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
    markerStyle,
    markerDigest,
    rulerMarks,
    formatClipTitle,
    keyIntent,
  };
})();

globalThis.PlayerCore = PlayerCore;
