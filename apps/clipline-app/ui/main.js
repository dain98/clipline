// DOM wiring + Tauri bridge. All player math and formatting lives in
// player-core.js (PlayerCore), which is unit-tested from Rust — keep this
// file to event plumbing and rendering.
const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);
const {
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
} = PlayerCore;

const video = $("video");
let currentClip = null;
let clipsCache = [];
let trimStart = 0;
let trimEnd = 0;
let dragging = null;
let rafId = 0;

function clipDuration() {
  if (Number.isFinite(video.duration) && video.duration > 0) return video.duration;
  if (currentClip && Number.isFinite(currentClip.duration_s)) return currentClip.duration_s;
  if (currentClip && currentClip.markers && Number.isFinite(currentClip.markers.duration_s)) {
    return currentClip.markers.duration_s;
  }
  return 0;
}

function clipMarkers() {
  return currentClip && currentClip.markers ? currentClip.markers.markers : [];
}

/* ---- sidebar: status, settings, library ---- */

function fillSettings(s) {
  $("set-capture").value = s.capture_mode;
  $("set-window").value = s.window_title ?? "";
  $("set-buffer").value = s.buffer_seconds;
  $("set-replay").value = s.replay_window_s;
  $("set-bitrate").value = s.bitrate_mbps;
  $("set-fps").value = s.fps;
  $("set-quota").value = s.disk_quota_gb;
  $("set-hotkey").value = s.hotkey;
  $("save-hotkey").textContent = s.hotkey;
  syncCaptureFields();
}

function readSettings() {
  return {
    capture_mode: $("set-capture").value,
    window_title: $("set-window").value,
    buffer_seconds: Number($("set-buffer").value),
    replay_window_s: Number($("set-replay").value),
    bitrate_mbps: Number($("set-bitrate").value),
    fps: Number($("set-fps").value),
    disk_quota_gb: Number($("set-quota").value),
    hotkey: $("set-hotkey").value,
  };
}

function syncCaptureFields() {
  $("set-window").disabled = $("set-capture").value !== "window_title";
}

async function refresh() {
  await Promise.all([refreshClips(), refreshStorage()]);
}

async function refreshStorage() {
  const s = await invoke("storage_status");
  $("storage-used").textContent = fmtBytes(s.total_bytes);
  $("storage-used").className = s.over_quota ? "warn" : "";
  $("storage-quota").textContent = s.quota_bytes == null ? "no limit" : fmtBytes(s.quota_bytes);
  $("storage-clips").textContent = s.clip_count;
}

async function refreshClips() {
  clipsCache = await invoke("list_clips");
  renderClips();
  if (currentClip) {
    const fresh = clipsCache.find((clip) => clip.path === currentClip.path);
    if (fresh) currentClip = fresh;
  }
}

// Clip names come from disk; build rows with textContent, never innerHTML.
function clipRow(c) {
  const el = document.createElement("div");
  el.className = "clip" + (currentClip && currentClip.path === c.path ? " active" : "");

  const meta = document.createElement("div");
  const name = document.createElement("div");
  name.className = "name";
  name.textContent = c.name;
  const markerCount = c.markers ? c.markers.markers.length : 0;
  if (markerCount) {
    const badge = document.createElement("span");
    badge.className = "badge";
    badge.textContent = markerCount;
    name.appendChild(badge);
  }
  const info = document.createElement("div");
  info.className = "info";
  info.textContent =
    `${fmtDur(c.duration_s)} · ${c.size_mb.toFixed(1)} MB · ` +
    fmtAgo(Date.now() / 1000, c.modified_unix);
  meta.append(name, info);

  const del = document.createElement("button");
  del.className = "del";
  del.title = "Delete clip";
  del.textContent = "✕";

  el.addEventListener("click", () => openClip(c));
  del.addEventListener("click", (ev) => {
    ev.stopPropagation();
    deleteClip(c.path);
  });

  el.append(meta, del);
  return el;
}

function renderClips() {
  const root = $("clips");
  root.replaceChildren();
  if (!clipsCache.length) {
    const hint = document.createElement("div");
    hint.className = "hint";
    hint.textContent = "no clips yet — press Alt+F10 while something plays";
    root.appendChild(hint);
    return;
  }
  for (const c of clipsCache) root.appendChild(clipRow(c));
}

/* ---- review player ---- */

function openClip(clip) {
  currentClip = clip;
  $("error").textContent = "";
  $("deck-status").textContent = "";
  $("stage-note").textContent = "loading…";
  $("pname").textContent = clip.name;
  $("pmeta").textContent = `${clip.size_mb.toFixed(1)} MB · ${clip.path}`;
  $("review-empty").hidden = true;
  $("review-viewer").hidden = false;
  video.src = convertFileSrc(clip.path);
  video.playbackRate = Number($("rate-select").value);
  setTrim(0, clip.duration_s ?? (clip.markers ? clip.markers.duration_s : 0));
  renderMarkers();
  renderClips();
  paintTimeline();
  video.play().catch(() => syncPlayState());
}

function closeReview() {
  cancelAnimationFrame(rafId);
  video.pause();
  video.removeAttribute("src");
  video.load();
  currentClip = null;
  $("review-viewer").hidden = true;
  $("review-empty").hidden = false;
  $("stage-note").textContent = "";
  $("timeline").querySelectorAll(".tick").forEach((t) => t.remove());
  renderClips();
}

function setTrim(start, end) {
  const next = resolveTrim(start, end, clipDuration());
  trimStart = next.start;
  trimEnd = next.end;
  $("trim-summary").textContent = trimSummary(trimStart, trimEnd);
  paintTimeline();
}

function paintTimeline() {
  const dur = clipDuration();
  const current = dur ? clampTime(video.currentTime || 0, dur) : 0;
  $("time-readout").textContent = `${fmtTenths(current)} / ${fmtTenths(dur)}`;
  $("playhead").style.left = `${percentFor(current, dur)}%`;
  $("dim-in").style.width = `${percentFor(trimStart, dur)}%`;
  $("dim-out").style.width = `${100 - percentFor(trimEnd, dur)}%`;
  $("handle-in").style.left = `${percentFor(trimStart, dur)}%`;
  $("handle-out").style.left = `${percentFor(trimEnd, dur)}%`;
}

// timeupdate fires ~4 Hz; animate the playhead per-frame while playing.
function animatePlayhead() {
  paintTimeline();
  if (!video.paused && !video.ended) rafId = requestAnimationFrame(animatePlayhead);
}

function renderMarkers() {
  const timeline = $("timeline");
  timeline.querySelectorAll(".tick").forEach((t) => t.remove());
  const dur = clipDuration();
  const markers = clipMarkers();
  for (const m of markers) {
    const tick = document.createElement("button");
    tick.className = "tick";
    tick.style.left = `${percentFor(m.t_s, dur)}%`;
    tick.title = `${m.kind}${m.subtype ? ` (${m.subtype})` : ""} — ${m.actor}${m.victim ? " → " + m.victim : ""} @ ${m.t_s.toFixed(1)}s`;
    tick.addEventListener("pointerdown", (ev) => ev.stopPropagation());
    tick.addEventListener("click", (ev) => {
      ev.stopPropagation();
      seekTo(m.t_s);
    });
    timeline.appendChild(tick);
  }
  $("marker-count").textContent = markerSummary(markers);
  $("prev-marker").disabled = !markers.length;
  $("next-marker").disabled = !markers.length;
}

function seekTo(time) {
  if (!currentClip) return;
  video.currentTime = clampTime(time, clipDuration());
  paintTimeline();
}

function seekBy(delta) {
  seekTo((video.currentTime || 0) + delta);
}

function togglePlay() {
  if (!currentClip) return;
  if (video.paused) video.play().catch(() => syncPlayState());
  else video.pause();
}

function syncPlayState() {
  $("play-toggle").textContent = video.paused ? "Play" : "Pause";
  $("play-toggle").setAttribute("aria-pressed", String(!video.paused));
}

function syncVolume() {
  $("mute-toggle").textContent = video.muted || video.volume === 0 ? "Unmute" : "Mute";
  $("volume-slider").value = String(video.muted ? 0 : video.volume);
}

function toggleMute() {
  if (video.muted || video.volume === 0) {
    video.muted = false;
    if (video.volume === 0) video.volume = 1;
  } else {
    video.muted = true;
  }
  syncVolume();
}

function jumpMarker(direction) {
  const markers = clipMarkers();
  const current = video.currentTime || 0;
  const target = direction > 0 ? nextMarker(markers, current) : prevMarker(markers, current);
  if (target) seekTo(target.t_s);
}

/* ---- timeline pointer interaction ---- */

function startDrag(kind, ev) {
  if (!currentClip) return;
  dragging = kind;
  $("timeline").setPointerCapture(ev.pointerId);
  moveDrag(ev);
}

function moveDrag(ev) {
  if (!dragging) return;
  const rect = $("timeline").getBoundingClientRect();
  const t = timelineTime(ev.clientX, rect.left, rect.width, clipDuration());
  if (dragging === "scrub") {
    seekTo(t);
  } else {
    const next = trimDrag(dragging, t, trimStart, trimEnd, clipDuration());
    setTrim(next.start, next.end);
  }
}

function endDrag() {
  dragging = null;
}

/* ---- clip actions ---- */

async function exportTrim() {
  if (!currentClip) return;
  $("error").textContent = "";
  $("export-clip").disabled = true;
  $("deck-status").textContent = "exporting…";
  try {
    const exported = await invoke("export_clip", {
      path: currentClip.path,
      startS: trimStart,
      endS: trimEnd,
    });
    $("deck-status").textContent =
      `exported ${exported.name} · keyframe-aligned ${fmtTenths(exported.aligned_start_s)} – ${fmtTenths(exported.aligned_end_s)}`;
    await refresh();
  } catch (e) {
    $("deck-status").textContent = "";
    $("error").textContent = e;
  } finally {
    $("export-clip").disabled = false;
  }
}

async function deleteClip(path = currentClip && currentClip.path) {
  if (!path) return;
  if (!confirm("Delete this clip from disk?")) return;
  try {
    await invoke("delete_clip", { path });
    $("notice").textContent = "clip deleted";
    $("error").textContent = "";
    if (currentClip && currentClip.path === path) closeReview();
    await refresh();
  } catch (e) {
    $("error").textContent = e;
  }
}

async function copyPath() {
  if (!currentClip) return;
  try {
    await navigator.clipboard.writeText(currentClip.path);
    $("deck-status").textContent = "source path copied";
  } catch (_) {
    $("deck-status").textContent = currentClip.path;
  }
}

/* ---- backend events ---- */

listen("status", (e) => {
  const s = e.payload;
  $("dot").className = "dot" + (s.recording ? " on" : "");
  $("state").textContent = s.recording ? "recording" : "stopped";
  $("buffered").textContent = s.buffered_s.toFixed(1) + " s";
  $("mb").textContent = s.buffered_mb.toFixed(1) + " MB";
  $("segs").textContent = s.segments;
});

listen("saved", (e) => {
  $("error").textContent = "";
  const s = e.payload;
  $("notice").textContent = s.gc_deleted
    ? `cleaned up ${s.gc_deleted} old clip${s.gc_deleted > 1 ? "s" : ""} (${fmtBytes(s.gc_freed_bytes)})`
    : `saved ${fmtDur(s.seconds)} replay`;
  refresh();
});

listen("error", (e) => { $("error").textContent = e.payload; });

/* ---- wiring ---- */

$("save").addEventListener("click", () => invoke("save_replay"));
$("set-capture").addEventListener("change", syncCaptureFields);
$("settings-save").addEventListener("click", async () => {
  $("settings-status").textContent = "";
  $("error").textContent = "";
  try {
    const saved = await invoke("save_settings", { settings: readSettings() });
    fillSettings(saved);
    $("settings-status").textContent = "saved";
    await refresh();
  } catch (e) {
    $("error").textContent = e;
  }
});

video.addEventListener("click", togglePlay);
video.addEventListener("play", () => {
  syncPlayState();
  cancelAnimationFrame(rafId);
  rafId = requestAnimationFrame(animatePlayhead);
});
video.addEventListener("pause", () => {
  syncPlayState();
  cancelAnimationFrame(rafId);
  paintTimeline();
});
video.addEventListener("timeupdate", paintTimeline);
video.addEventListener("volumechange", syncVolume);
video.addEventListener("loadedmetadata", () => {
  $("stage-note").textContent = `${video.videoWidth}x${video.videoHeight} · ${fmtDur(video.duration)}`;
  if (currentClip) {
    $("pmeta").textContent = `${fmtDur(video.duration)} · ${currentClip.size_mb.toFixed(1)} MB · ${currentClip.path}`;
    setTrim(0, video.duration);
    renderMarkers();
  }
});
video.addEventListener("error", () => {
  const e = video.error;
  $("stage-note").textContent = `load error ${e ? e.code : "?"}`;
});

$("play-toggle").addEventListener("click", togglePlay);
$("seek-back").addEventListener("click", () => seekBy(-5));
$("seek-forward").addEventListener("click", () => seekBy(5));
$("prev-marker").addEventListener("click", () => jumpMarker(-1));
$("next-marker").addEventListener("click", () => jumpMarker(1));
$("mute-toggle").addEventListener("click", toggleMute);
$("rate-select").addEventListener("change", () => {
  video.playbackRate = Number($("rate-select").value);
});
$("volume-slider").addEventListener("input", () => {
  video.volume = Number($("volume-slider").value);
  video.muted = video.volume === 0;
});

$("export-clip").addEventListener("click", exportTrim);
$("delete-clip").addEventListener("click", () => deleteClip());
$("copy-path").addEventListener("click", copyPath);
$("close-review").addEventListener("click", closeReview);

$("timeline").addEventListener("pointerdown", (ev) => {
  if (ev.target === $("handle-in")) startDrag("in", ev);
  else if (ev.target === $("handle-out")) startDrag("out", ev);
  else startDrag("scrub", ev);
});
$("timeline").addEventListener("pointermove", moveDrag);
$("timeline").addEventListener("pointerup", endDrag);
$("timeline").addEventListener("pointercancel", endDrag);
$("timeline").addEventListener("lostpointercapture", endDrag);

document.addEventListener("keydown", (ev) => {
  if (!currentClip) return;
  const tag = ev.target && ev.target.tagName;
  if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") return;
  const intent = keyIntent(ev.code, ev.shiftKey);
  if (!intent) return;
  ev.preventDefault();
  switch (intent.kind) {
    case "toggle-play": togglePlay(); break;
    case "seek-by": seekBy(intent.seconds); break;
    case "set-in": setTrim(video.currentTime || 0, trimEnd); break;
    case "set-out": setTrim(trimStart, video.currentTime || 0); break;
    case "next-marker": jumpMarker(1); break;
    case "prev-marker": jumpMarker(-1); break;
    case "close": closeReview(); break;
  }
});

/* ---- boot ---- */

$("review-viewer").hidden = true;
syncPlayState();
syncVolume();
invoke("get_settings").then(fillSettings).catch((e) => $("error").textContent = e);
refresh();
