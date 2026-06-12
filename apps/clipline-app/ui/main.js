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
  overlayVisible,
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
  el.title = c.name;

  const meta = document.createElement("div");
  const name = document.createElement("div");
  name.className = "name";
  const when = new Date(c.modified_unix * 1000);
  name.textContent = formatClipTitle(
    when.getMonth(), when.getDate(), when.getHours(), when.getMinutes());
  const info = document.createElement("div");
  info.className = "info";
  const digest = markerDigest(c.markers ? c.markers.markers : []);
  info.textContent =
    `${fmtDur(c.duration_s)} · ${c.size_mb.toFixed(1)} MB · ` +
    fmtAgo(Date.now() / 1000, c.modified_unix) +
    (digest ? ` · ${digest}` : "");
  meta.append(name, info);

  const del = document.createElement("button");
  del.className = "del";
  del.title = "Delete clip";
  // Static markup, no clip data — innerHTML is safe here.
  del.innerHTML =
    '<svg viewBox="0 0 24 24"><path d="M9 3v1H4v2h16V4h-5V3H9zM6 8v11a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2V8H6zm3 2h2v9H9v-9zm4 0h2v9h-2v-9z"/></svg>';

  // Clicking the open clip's row again closes it (there is no Close button).
  el.addEventListener("click", () => {
    if (currentClip && currentClip.path === c.path) closeReview();
    else openClip(c);
  });
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
  for (const group of sessionGroups(clipsCache)) {
    const head = document.createElement("div");
    head.className = "session-head";
    head.textContent = group.label;
    root.appendChild(head);
    for (const c of group.clips) root.appendChild(clipRow(c));
  }
}

/* ---- review player ---- */

function openClip(clip) {
  currentClip = clip;
  $("error").textContent = "";
  $("deck-status").textContent = "";
  $("stage-note").textContent = "loading…";
  $("pname").textContent = clip.name;
  $("pmeta").textContent = `${clip.size_mb.toFixed(1)} MB · ${clip.path}`;
  settingsOpen = false;
  updateViews();
  video.src = convertFileSrc(clip.path);
  video.playbackRate = Number($("rate-select").value);
  setTrim(0, clip.duration_s ?? (clip.markers ? clip.markers.duration_s : 0));
  renderMarkers();
  renderRuler();
  renderClips();
  paintTimeline();
  noteActivity();
  video.play().catch(() => syncPlayState());
}

function closeReview() {
  cancelAnimationFrame(rafId);
  video.pause();
  video.removeAttribute("src");
  video.load();
  currentClip = null;
  updateViews();
  $("stage-note").textContent = "";
  $("timeline").querySelectorAll(".tick").forEach((t) => t.remove());
  renderClips();
}

/* ---- main pane views: empty / player / settings ---- */

let settingsOpen = false;

function updateViews() {
  $("settings-page").hidden = !settingsOpen;
  $("review-viewer").hidden = settingsOpen || !currentClip;
  $("review-empty").hidden = settingsOpen || !!currentClip;
}

function toggleSettings(open = !settingsOpen) {
  settingsOpen = open;
  // The clip survives the round-trip; just don't play behind the page.
  if (settingsOpen && !video.paused) video.pause();
  updateViews();
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
// The same loop re-evaluates overlay fade (no timers to manage).
function animatePlayhead() {
  paintTimeline();
  updateOverlay();
  if (!video.paused && !video.ended) rafId = requestAnimationFrame(animatePlayhead);
}

function renderMarkers() {
  const timeline = $("timeline");
  timeline.querySelectorAll(".tick").forEach((t) => t.remove());
  const dur = clipDuration();
  const markers = clipMarkers();
  for (const m of markers) {
    const tick = document.createElement("button");
    const style = markerStyle(m.kind);
    tick.className = `tick tick-${style.cls}`;
    tick.textContent = style.glyph;
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

function renderRuler() {
  const root = $("ruler");
  root.replaceChildren();
  for (const mark of rulerMarks(clipDuration(), 8)) {
    const span = document.createElement("span");
    span.style.left = `${percentFor(mark.t, clipDuration())}%`;
    span.textContent = mark.label;
    root.appendChild(span);
  }
}

function toggleRail() {
  document.querySelector(".app").classList.toggle("rail");
}

// Rapid seeks (scrubbing) must not pile up: WebView2 stops painting frames
// when a new seek lands while the previous one is in flight. Issue one seek
// at a time and chain the latest target from the `seeked` event.
let pendingSeek = null;

function seekTo(time) {
  if (!currentClip) return;
  const t = clampTime(time, clipDuration());
  if (video.seeking) {
    pendingSeek = t;
  } else {
    pendingSeek = null;
    video.currentTime = t;
  }
  paintTimeline();
}

video.addEventListener("seeked", () => {
  if (pendingSeek != null) {
    const t = pendingSeek;
    pendingSeek = null;
    video.currentTime = t;
  }
  paintTimeline();
});

function seekBy(delta) {
  seekTo((video.currentTime || 0) + delta);
}

function togglePlay() {
  if (!currentClip) return;
  if (video.paused) video.play().catch(() => syncPlayState());
  else video.pause();
}

function syncPlayState() {
  $("play-toggle").classList.toggle("playing", !video.paused);
  $("play-toggle").setAttribute("aria-pressed", String(!video.paused));
  updateOverlay();
}

function syncVolume() {
  $("mute-toggle").classList.toggle("muted", video.muted || video.volume === 0);
  $("volume-slider").value = String(video.muted ? 0 : video.volume);
}

/* ---- overlay visibility (PlayerCore.overlayVisible policy) ---- */

let lastActivityMs = 0;

function noteActivity() {
  lastActivityMs = performance.now();
  updateOverlay();
}

function updateOverlay() {
  const idleMs = performance.now() - lastActivityMs;
  document
    .querySelector(".stage")
    .classList.toggle("idle", !overlayVisible(video.paused, idleMs));
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

let resumeAfterDrag = false;

function startDrag(kind, ev) {
  if (!currentClip) return;
  dragging = kind;
  // Scrub paused so every pointer position shows its frame, then restore.
  resumeAfterDrag = !video.paused;
  if (resumeAfterDrag) video.pause();
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
    // The playhead rides the dragged edge — you trim on the frame you see.
    seekTo(dragging === "in" ? next.start : next.end);
  }
}

function endDrag() {
  if (!dragging) return;
  dragging = null;
  if (resumeAfterDrag) {
    resumeAfterDrag = false;
    video.play().catch(() => syncPlayState());
  }
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

// In-app modal — the native browser prompt renders "tauri.localhost says".
function confirmDelete(name) {
  return new Promise((resolve) => {
    const dlg = $("confirm-dialog");
    $("confirm-detail").textContent = name;
    const finish = (ok) => {
      dlg.removeEventListener("close", onClose);
      if (dlg.open) dlg.close();
      resolve(ok);
    };
    const onClose = () => finish(false); // Esc / backdrop paths
    dlg.addEventListener("close", onClose);
    $("confirm-cancel").onclick = () => finish(false);
    $("confirm-accept").onclick = () => finish(true);
    dlg.showModal();
  });
}

async function deleteClip(path = currentClip && currentClip.path) {
  if (!path) return;
  const name = path.split(/[\\/]/).pop();
  if (!(await confirmDelete(name))) return;
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

async function openFolder() {
  if (!currentClip) return;
  try {
    await invoke("reveal_clip", { path: currentClip.path });
  } catch (e) {
    $("error").textContent = e;
  }
}

/* ---- backend events ---- */

listen("status", (e) => {
  const s = e.payload;
  $("dot").className = "dot" + (s.recording ? " on" : "");
  $("rail-dot").className = "dot" + (s.recording ? " on" : "");
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
    renderRuler();
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
$("open-folder").addEventListener("click", openFolder);

$("sidebar-toggle").addEventListener("click", toggleRail);
$("rail-save").addEventListener("click", () => invoke("save_replay"));
$("rail-settings").addEventListener("click", () => toggleSettings());
$("open-settings").addEventListener("click", () => toggleSettings());
$("settings-close").addEventListener("click", () => toggleSettings(false));

document.querySelectorAll("#settings-tabs .tab").forEach((tab) => {
  tab.addEventListener("click", () => {
    document
      .querySelectorAll("#settings-tabs .tab")
      .forEach((t) => t.classList.toggle("active", t === tab));
    document.querySelectorAll(".settings-section").forEach((s) => {
      s.hidden = s.dataset.section !== tab.dataset.tab;
    });
  });
});

$("timeline").addEventListener("pointerdown", (ev) => {
  if (ev.target === $("handle-in")) startDrag("in", ev);
  else if (ev.target === $("handle-out")) startDrag("out", ev);
  else startDrag("scrub", ev);
});
$("timeline").addEventListener("pointermove", moveDrag);

const stage = document.querySelector(".stage");
stage.addEventListener("pointermove", noteActivity);
stage.addEventListener("pointerdown", noteActivity);
stage.addEventListener("pointerleave", () => {
  // Leaving the stage while playing hides the bar immediately.
  lastActivityMs = -Infinity;
  updateOverlay();
});
$("timeline").addEventListener("pointerup", endDrag);
$("timeline").addEventListener("pointercancel", endDrag);
$("timeline").addEventListener("lostpointercapture", endDrag);

document.addEventListener("keydown", (ev) => {
  if ($("confirm-dialog").open) return; // the dialog owns the keyboard
  if (ev.code === "Escape" && settingsOpen) {
    ev.preventDefault();
    toggleSettings(false);
    return;
  }
  if (settingsOpen) return; // player shortcuts are inert behind the page
  if (!currentClip) return;
  const tag = ev.target && ev.target.tagName;
  if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") return;
  const intent = keyIntent(ev.code, ev.shiftKey);
  if (!intent) return;
  ev.preventDefault();
  noteActivity();
  switch (intent.kind) {
    case "toggle-play": togglePlay(); break;
    case "seek-by": seekBy(intent.seconds); break;
    case "set-in": setTrim(video.currentTime || 0, trimEnd); break;
    case "set-out": setTrim(trimStart, video.currentTime || 0); break;
    case "next-marker": jumpMarker(1); break;
    case "prev-marker": jumpMarker(-1); break;
    case "toggle-focus": toggleRail(); break;
    case "close": closeReview(); break;
  }
});

/* ---- boot ---- */

updateViews();
syncPlayState();
syncVolume();
invoke("get_settings").then(fillSettings).catch((e) => $("error").textContent = e);
refresh();
