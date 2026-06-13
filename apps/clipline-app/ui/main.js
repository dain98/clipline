// DOM wiring + Tauri bridge. All player math and formatting lives in
// player-core.js (PlayerCore), which is unit-tested from Rust — keep this
// file to event plumbing and rendering.
const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const appWindow = window.__TAURI__.window.getCurrentWindow();
const $ = (id) => document.getElementById(id);

// Custom window chrome — the native title bar is disabled (decorations: false).
$("win-min").addEventListener("click", () => appWindow.minimize());
$("win-max").addEventListener("click", () => appWindow.toggleMaximize());
$("win-close").addEventListener("click", () => appWindow.close());
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
  clipKind,
  keyIntent,
  hotkeyFromKeyEvent,
  displayMapLayout,
  displayMapHeight,
  regionForDisplay,
  clampRegionToDisplay,
  alignRegion,
  settingDurationLabel,
  recordingQualityPreset,
  qualityIndexForBitrate,
  smoothnessPreset,
  smoothnessIndexForFps,
  captureSourceLabel,
} = PlayerCore;

const video = $("video");
const stage = document.querySelector(".stage");
const stageFrame = $("stage-frame");
let currentClip = null;
let clipsCache = [];
let currentSettings = null;
let recordingActive = true;
let displays = [];
let audioDevices = { outputs: [], inputs: [] };
let videoEncoders = [];
let customGames = [];
let gameWindows = [];
let activeDetectedGame = null;
let regionState = { display_id: null, x: 0, y: 0, width: 1920, height: 1080 };
let regionLayout = null;
let regionDrag = null;
let regionMenuDisplayId = null;
let micTestRunning = false;
let micAudioContext = null;
let micAudioCursor = 0;
let micAudioSources = [];
let hotkeyCaptureActive = false;
let trimStart = 0;
let trimEnd = 0;
let dragging = null;
let rafId = 0;
const MIC_MONITOR_START_DELAY_S = 0.02;
const MIC_MONITOR_MAX_LATENCY_S = 0.25;

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

function videoAspect() {
  return video.videoWidth > 0 && video.videoHeight > 0
    ? video.videoWidth / video.videoHeight
    : 16 / 9;
}

function updateStageFrame() {
  const bounds = stage.getBoundingClientRect();
  if (bounds.width <= 0 || bounds.height <= 0) return;
  const aspect = videoAspect();
  let width = bounds.width;
  let height = width / aspect;
  if (height > bounds.height) {
    height = bounds.height;
    width = height * aspect;
  }
  stageFrame.style.width = `${Math.max(1, Math.floor(width))}px`;
  stageFrame.style.height = `${Math.max(1, Math.floor(height))}px`;
}

/* ---- sidebar: status, settings, library ---- */

function fillSettings(s) {
  const audio = { ...defaultAudioSettings(), ...(s.audio || {}) };
  const replayStorage = { ...defaultReplayStorageSettings(), ...(s.replay_storage || {}) };
  const games = { ...defaultGameSettings(), ...(s.games || {}) };
  customGames = (games.custom_games || []).map(normalizeCustomGame);
  currentSettings = {
    ...s,
    audio,
    replay_storage: replayStorage,
    games: { ...games, custom_games: customGames.map((game) => ({ ...game })) },
  };
  $("set-capture").value = captureSettingsMode(s.capture_mode);
  regionState = s.capture_region ?? regionState;
  $("set-games-auto-detect").checked = !!games.auto_detect;
  $("set-output-enabled").checked = !!audio.output_enabled;
  $("set-output-volume").value = String(Number.isFinite(audio.output_volume) ? audio.output_volume : 1);
  $("set-mic-enabled").checked = !!audio.mic_enabled;
  $("set-mic-volume").value = String(Number.isFinite(audio.mic_volume) ? audio.mic_volume : 1);
  $("set-mic-mono").checked = (audio.mic_channels || "mono") === "mono";
  $("set-buffer").value = Number(s.buffer_seconds) || ((Number(s.replay_window_s) || 60) + 15);
  $("set-replay").value = Math.min(120, Number(s.replay_window_s) || 60);
  $("set-encoder").value = s.video_encoder || "auto";
  $("set-bitrate").value = qualityIndexForBitrate(s.bitrate_mbps);
  $("set-fps").value = smoothnessIndexForFps(s.fps);
  $("set-quota").value = s.disk_quota_gb;
  $("set-media-dir").value = s.media_dir ?? "";
  $("set-replay-disk-enabled").checked = replayStorage.mode === "disk";
  $("set-replay-disk-dir").value = replayStorage.disk_dir || "";
  $("set-replay-disk-quota").value = replayStorage.disk_quota_gb ?? 2;
  $("set-replay-disk-ack").checked = !!replayStorage.disk_acknowledged;
  $("set-hotkey").value = s.hotkey;
  $("save-hotkey").textContent = s.hotkey;
  endHotkeyCapture("Click the field to record a new shortcut.");
  syncCaptureFields();
  renderAudioDeviceSelects();
  renderVideoEncoderSelect();
  syncAudioFields();
  syncRecordingFields();
  syncReplayStorageFields();
  renderCustomGames();
  updateGameDetectionStatus();
  updateCaptureStatus();
}

function readSettings() {
  const replay = Number($("set-replay").value);
  return {
    capture_mode: $("set-capture").value,
    window_title: "",
    capture_region: regionState,
    games: {
      auto_detect: $("set-games-auto-detect").checked,
      custom_games: customGames.map((game) => ({ ...game })),
    },
    audio: {
      output_enabled: $("set-output-enabled").checked,
      output_device_id: selectedDeviceId("set-output-device"),
      output_volume: Number($("set-output-volume").value),
      mic_enabled: $("set-mic-enabled").checked,
      mic_device_id: selectedDeviceId("set-mic-device"),
      mic_volume: Number($("set-mic-volume").value),
      mic_channels: $("set-mic-mono").checked ? "mono" : "stereo",
    },
    // Ring holds the save window plus 15 s headroom (mirrors BUFFER_HEADROOM_S
    // in settings.rs) - not a fixed 2 minutes.
    buffer_seconds: replay + 15,
    replay_window_s: replay,
    video_encoder: $("set-encoder").value,
    bitrate_mbps: recordingQualityPreset(Number($("set-bitrate").value)).bitrate,
    fps: smoothnessPreset(Number($("set-fps").value)).fps,
    disk_quota_gb: Number($("set-quota").value),
    media_dir: $("set-media-dir").value.trim(),
    replay_storage: {
      mode: $("set-replay-disk-enabled").checked ? "disk" : "memory",
      disk_dir: $("set-replay-disk-dir").value.trim(),
      disk_quota_gb: Number($("set-replay-disk-quota").value),
      disk_acknowledged: $("set-replay-disk-ack").checked,
    },
    hotkey: $("set-hotkey").value,
  };
}

function defaultAudioSettings() {
  return {
    output_enabled: true,
    output_device_id: null,
    output_volume: 1,
    mic_enabled: false,
    mic_device_id: null,
    mic_volume: 1,
    mic_channels: "mono",
  };
}

function defaultReplayStorageSettings() {
  return {
    mode: "memory",
    disk_dir: "",
    disk_quota_gb: 2,
    disk_acknowledged: false,
  };
}

function defaultGameSettings() {
  return {
    auto_detect: true,
    custom_games: [],
  };
}

function normalizeGameRecordingMode(mode) {
  return mode === "full_session" ? "full_session" : "replays_only";
}

function normalizeCustomGame(game) {
  return {
    id: String(game.id || `custom-${Date.now()}`),
    name: String(game.name || game.exe_name || game.window_title || "Custom game").trim(),
    enabled: game.enabled !== false,
    exe_name: String(game.exe_name || "").trim(),
    process_path: game.process_path ? String(game.process_path).trim() : null,
    window_title: String(game.window_title || "").trim(),
    recording_mode: normalizeGameRecordingMode(game.recording_mode),
  };
}

function captureSettingsMode(mode) {
  return mode === "display_region" ? "display_region" : "primary_monitor";
}

function syncCaptureFields() {
  const mode = $("set-capture").value;
  $("capture-region-editor").hidden = mode !== "display_region";
  if (mode === "display_region") renderRegionEditor();
}

function syncRecordingFields() {
  const replay = Number($("set-replay").value);
  const encoder = selectedVideoEncoder();
  const quality = recordingQualityPreset(Number($("set-bitrate").value));
  const smoothness = smoothnessPreset(Number($("set-fps").value));
  syncRangeProgress($("set-replay"));
  syncRangeProgress($("set-bitrate"));
  syncRangeProgress($("set-fps"));
  $("replay-summary").textContent = `Save Replay writes the last ${settingDurationLabel(replay)}.`;
  $("replay-summary").className = "setting-summary";
  $("encoder-summary").textContent =
    encoder.id === "auto"
      ? "Clipline chooses the best available H.264 GPU encoder."
      : `${encoder.name} is used for new recordings.`;
  $("quality-summary").textContent = `${quality.label} quality - ${quality.hint}.`;
  $("fps-summary").textContent = `${smoothness.label} - ${smoothness.hint}.`;
  syncReplayStorageFields();
}

function syncReplayStorageFields() {
  const enabled = $("set-replay-disk-enabled").checked;
  const fields = $("replay-disk-fields");
  fields.hidden = !enabled;
  const quality = recordingQualityPreset(Number($("set-bitrate").value));
  const gbPerHour = quality.bitrate * 1_000_000 / 8 * 3600 / (1000 ** 3);
  $("replay-disk-estimate").textContent =
    `${quality.bitrate} Mbps: about ${gbPerHour.toFixed(quality.bitrate >= 40 ? 0 : 1)} GB/hour written while recording.`;
  for (const id of ["set-replay-disk-dir", "choose-replay-cache-folder", "set-replay-disk-quota", "set-replay-disk-ack"]) {
    $(id).disabled = !enabled;
  }
}

function volumeLabel(value) {
  const pct = Math.round(Math.max(0, Math.min(2, Number(value) || 0)) * 100);
  return `${pct}%`;
}

function syncRangeProgress(input) {
  const min = Number(input.min || 0);
  const max = Number(input.max || 100);
  const value = Number(input.value || min);
  const pct = max > min ? ((value - min) / (max - min)) * 100 : 0;
  input.style.setProperty("--range-progress", `${Math.max(0, Math.min(100, pct)).toFixed(2)}%`);
}

function syncAllRangeProgress() {
  document.querySelectorAll("input[type='range']").forEach(syncRangeProgress);
}

function selectedDeviceId(id) {
  const value = $(id).value;
  return value ? value : null;
}

function fillDeviceSelect(id, devices, defaultLabel, selectedId) {
  const select = $(id);
  const selected = selectedId || "";
  select.replaceChildren();
  const def = document.createElement("option");
  def.value = "";
  def.textContent = defaultLabel;
  select.appendChild(def);
  for (const device of devices) {
    const opt = document.createElement("option");
    opt.value = device.id;
    opt.textContent = device.name + (device.is_default ? " (default)" : "");
    select.appendChild(opt);
  }
  if (selected && !devices.some((device) => device.id === selected)) {
    const stale = document.createElement("option");
    stale.value = selected;
    stale.textContent = "Unavailable device";
    select.appendChild(stale);
  }
  select.value = selected;
}

function renderAudioDeviceSelects() {
  const audio = currentSettings && currentSettings.audio ? currentSettings.audio : defaultAudioSettings();
  fillDeviceSelect("set-output-device", audioDevices.outputs, "Default output device", audio.output_device_id);
  fillDeviceSelect("set-mic-device", audioDevices.inputs, "Default microphone", audio.mic_device_id);
}

function renderVideoEncoderSelect() {
  const select = $("set-encoder");
  const selected = currentSettings && currentSettings.video_encoder ? currentSettings.video_encoder : "auto";
  select.replaceChildren();
  const automatic = document.createElement("option");
  automatic.value = "auto";
  automatic.textContent = "Automatic (recommended)";
  select.appendChild(automatic);
  for (const encoder of videoEncoders) {
    const opt = document.createElement("option");
    opt.value = encoder.id;
    opt.textContent = encoder.name;
    select.appendChild(opt);
  }
  if (selected !== "auto" && !videoEncoders.some((encoder) => encoder.id === selected)) {
    const stale = document.createElement("option");
    stale.value = selected;
    stale.textContent = "Unavailable encoder";
    select.appendChild(stale);
  }
  select.value = selected;
}

function selectedVideoEncoder() {
  const id = $("set-encoder").value || "auto";
  if (id === "auto") return { id, name: "Automatic (recommended)" };
  return videoEncoders.find((encoder) => encoder.id === id) || { id, name: "Unavailable encoder" };
}

function syncAudioFields() {
  const outputEnabled = $("set-output-enabled").checked;
  $("set-output-device").disabled = !outputEnabled;
  $("set-output-volume").disabled = !outputEnabled;
  $("set-mic-device").disabled = micTestRunning;
  $("set-mic-volume").disabled = micTestRunning;
  $("set-mic-mono").disabled = micTestRunning;
  $("test-mic").disabled = false;
  $("test-mic").textContent = micTestRunning ? "Stop testing" : "Test mic";
  syncRangeProgress($("set-output-volume"));
  syncRangeProgress($("set-mic-volume"));
  $("output-volume-summary").textContent = volumeLabel($("set-output-volume").value);
  $("mic-volume-summary").textContent = volumeLabel($("set-mic-volume").value);
}

function setMicTestStatus(message, level = 0) {
  $("mic-test-status").textContent = message;
  $("mic-meter-fill").style.width = `${Math.round(Math.max(0, Math.min(1, level)) * 100)}%`;
}

function micMeterLevel(result) {
  const peak = Math.max(0, Number(result.peak) || 0);
  const rms = Math.max(0, Number(result.rms) || 0);
  return Math.min(1, Math.sqrt(Math.max(peak, rms * 3)));
}

function ensureMicAudioContext() {
  const AudioContextCtor = window.AudioContext || window.webkitAudioContext;
  if (!AudioContextCtor) throw new Error("Web Audio is unavailable");
  if (!micAudioContext || micAudioContext.state === "closed") {
    micAudioContext = new AudioContextCtor({ sampleRate: 48000 });
  }
  return micAudioContext;
}

async function startMicPlayback() {
  const ctx = ensureMicAudioContext();
  if (ctx.state === "suspended") await ctx.resume();
  micAudioCursor = ctx.currentTime + 0.04;
}

function stopMicPlayback() {
  for (const source of micAudioSources) {
    try {
      source.stop();
    } catch (_) {
      // Already ended.
    }
  }
  micAudioSources = [];
  micAudioCursor = 0;
}

function playMicSamples(samples) {
  if (!micTestRunning || !samples || samples.length < 2) return;
  const ctx = ensureMicAudioContext();
  const frames = Math.floor(samples.length / 2);
  const buffer = ctx.createBuffer(2, frames, 48000);
  const left = buffer.getChannelData(0);
  const right = buffer.getChannelData(1);
  for (let i = 0; i < frames; i += 1) {
    left[i] = Math.max(-1, Math.min(1, samples[i * 2] / 32768));
    right[i] = Math.max(-1, Math.min(1, samples[i * 2 + 1] / 32768));
  }
  const source = ctx.createBufferSource();
  source.buffer = buffer;
  source.connect(ctx.destination);
  const nextStart = ctx.currentTime + MIC_MONITOR_START_DELAY_S;
  if (
    !micAudioCursor ||
    micAudioCursor < nextStart ||
    micAudioCursor - ctx.currentTime > MIC_MONITOR_MAX_LATENCY_S
  ) {
    micAudioCursor = nextStart;
  }
  const startAt = micAudioCursor;
  source.start(startAt);
  micAudioCursor = startAt + buffer.duration;
  micAudioSources.push(source);
  source.onended = () => {
    micAudioSources = micAudioSources.filter((item) => item !== source);
  };
}

function stopMicTestUi(message = "stopped") {
  micTestRunning = false;
  stopMicPlayback();
  syncAudioFields();
  setMicTestStatus(message, 0);
}

async function testMic() {
  $("error").textContent = "";
  if (micTestRunning) {
    try {
      await invoke("stop_microphone_test");
    } catch (e) {
      $("error").textContent = e;
    }
    stopMicTestUi("stopped");
    return;
  }

  micTestRunning = true;
  syncAudioFields();
  setMicTestStatus("listening", 0);
  try {
    await startMicPlayback();
    await invoke("start_microphone_test", {
      deviceId: selectedDeviceId("set-mic-device"),
      volume: Number($("set-mic-volume").value),
      mono: $("set-mic-mono").checked,
    });
  } catch (e) {
    stopMicTestUi("error");
    $("error").textContent = e;
  }
}

function updateCaptureStatus() {
  const source =
    activeDetectedGame && activeDetectedGame.active
      ? `Game: ${activeDetectedGame.name}`
      : captureSourceLabel(currentSettings || { capture_mode: "primary_monitor" });
  $("capture-status-label").textContent = recordingActive ? `Capturing ${source}` : "Recording stopped";
  $("capture-status").classList.toggle("stopped", !recordingActive);
  $("capture-status").setAttribute("aria-pressed", String(recordingActive));
  $("capture-status").title = recordingActive ? "Stop recording" : `Start ${source} recording`;
  $("rail-status").title = $("capture-status").title;
  $("save").disabled = !recordingActive;
  $("rail-save").disabled = !recordingActive;
}

async function toggleRecording() {
  const next = !recordingActive;
  $("capture-status").disabled = true;
  $("rail-status").disabled = true;
  try {
    recordingActive = await invoke("set_recording", { recording: next });
    updateCaptureStatus();
  } catch (e) {
    $("error").textContent = e;
  } finally {
    $("capture-status").disabled = false;
    $("rail-status").disabled = false;
  }
}

function setHotkeyStatus(message, state = "") {
  const status = $("hotkey-status");
  status.textContent = message;
  status.dataset.state = state;
}

function beginHotkeyCapture() {
  hotkeyCaptureActive = true;
  $("set-hotkey").classList.add("recording");
  setHotkeyStatus("Press F1-F11 or F13-F24. Ctrl/Alt/Shift are optional.", "recording");
}

function endHotkeyCapture(message = "Click the field to record a new shortcut.", state = "") {
  hotkeyCaptureActive = false;
  $("set-hotkey").classList.remove("recording");
  setHotkeyStatus(message, state);
}

function recordHotkey(ev) {
  if (!hotkeyCaptureActive) beginHotkeyCapture();
  ev.preventDefault();
  ev.stopPropagation();

  const result = hotkeyFromKeyEvent(ev);
  switch (result.kind) {
    case "captured":
      $("set-hotkey").value = result.value;
      endHotkeyCapture("Ready to save.", "ready");
      break;
    case "pending":
      setHotkeyStatus(result.message, "recording");
      break;
    case "cancel":
      endHotkeyCapture("Shortcut unchanged.", "");
      $("set-hotkey").blur();
      break;
    case "invalid":
      setHotkeyStatus(result.message, "error");
      break;
  }
}

function primaryDisplay() {
  return displays.find((d) => d.is_primary) || displays[0] || null;
}

function activeDisplay() {
  return displays.find((d) => d.id === regionState.display_id) || primaryDisplay();
}

function menuDisplay() {
  return displays.find((d) => d.id === regionMenuDisplayId) || activeDisplay();
}

function setRegion(next) {
  const display = displays.find((d) => d.id === next.display_id) || activeDisplay();
  regionState = display
    ? clampRegionToDisplay({ ...next, display_id: display.id }, display)
    : {
        display_id: next.display_id ?? null,
        x: Math.round(next.x || 0),
        y: Math.round(next.y || 0),
        width: Math.max(2, Math.round(next.width || 2)),
        height: Math.max(2, Math.round(next.height || 2)),
      };
  renderRegionEditor();
}

async function loadDisplays() {
  try {
    displays = await invoke("list_displays");
    if (!regionState.display_id && displays.length) {
      regionState = regionForDisplay(primaryDisplay());
    }
    renderRegionEditor();
  } catch (e) {
    $("region-display-label").textContent = "display list unavailable";
    $("error").textContent = e;
  }
}

async function loadAudioDevices() {
  try {
    audioDevices = await invoke("list_audio_devices");
    renderAudioDeviceSelects();
  } catch (e) {
    $("error").textContent = e;
  }
}

async function loadVideoEncoders() {
  try {
    videoEncoders = await invoke("list_video_encoders");
    renderVideoEncoderSelect();
    if (currentSettings) syncRecordingFields();
  } catch (e) {
    videoEncoders = [];
    renderVideoEncoderSelect();
    if (currentSettings) syncRecordingFields();
    $("error").textContent = e;
  }
}

function gameNameFromWindow(win) {
  const exe = String(win.exe_name || "").replace(/\.exe$/i, "");
  return exe || String(win.title || "Custom game").trim() || "Custom game";
}

function customGameId(name) {
  const slug = String(name || "game")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 28) || "game";
  return `custom-${slug}-${Date.now()}`;
}

function gameRecordingModeControl(game, index) {
  const control = document.createElement("div");
  control.className = "segmented-control custom-game-mode";
  control.setAttribute("role", "radiogroup");
  control.setAttribute("aria-label", `${game.name} recording mode`);
  const selectedMode = normalizeGameRecordingMode(game.recording_mode);
  [
    ["replays_only", "Replays only"],
    ["full_session", "Full session"],
  ].forEach(([value, label]) => {
    const option = document.createElement("label");
    const input = document.createElement("input");
    input.type = "radio";
    input.name = `custom-game-recording-mode-${index}`;
    input.value = value;
    input.checked = selectedMode === value;
    input.addEventListener("change", () => {
      if (input.checked) {
        customGames[index] = { ...customGames[index], recording_mode: value };
      }
    });
    const text = document.createElement("span");
    text.textContent = label;
    option.append(input, text);
    control.appendChild(option);
  });
  return control;
}

function renderCustomGames() {
  const root = $("custom-games");
  root.replaceChildren();
  if (!customGames.length) {
    const empty = document.createElement("div");
    empty.className = "hint";
    empty.textContent = "no custom games saved";
    root.appendChild(empty);
    return;
  }
  customGames.forEach((game, index) => {
    const row = document.createElement("div");
    row.className = "custom-game";

    const enabled = document.createElement("label");
    enabled.className = "check-line";
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = game.enabled;
    checkbox.addEventListener("change", () => {
      customGames[index] = { ...customGames[index], enabled: checkbox.checked };
    });
    enabled.appendChild(checkbox);

    const meta = document.createElement("div");
    meta.className = "custom-game-meta";
    const name = document.createElement("strong");
    name.textContent = game.name;
    const info = document.createElement("span");
    info.textContent =
      `${game.exe_name || "window title"} · ${game.window_title || game.process_path || "custom rule"}`;
    meta.append(name, info);

    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "custom-game-remove";
    remove.title = "Remove custom game";
    remove.textContent = "×";
    remove.addEventListener("click", () => {
      customGames.splice(index, 1);
      renderCustomGames();
    });

    row.append(enabled, meta, remove, gameRecordingModeControl(game, index));
    root.appendChild(row);
  });
}

function renderGameWindows() {
  const root = $("game-window-list");
  root.replaceChildren();
  if (!gameWindows.length) {
    const empty = document.createElement("div");
    empty.className = "hint";
    empty.textContent = "no running windows found";
    root.appendChild(empty);
    return;
  }
  for (const win of gameWindows) {
    const row = document.createElement("button");
    row.type = "button";
    row.className = "game-window";
    const title = document.createElement("strong");
    title.textContent = win.title;
    const meta = document.createElement("span");
    meta.textContent =
      `${win.exe_name || "unknown process"} · PID ${win.process_id}` +
      (win.exe_path ? ` · ${win.exe_path}` : "");
    row.append(title, meta);
    row.addEventListener("click", () => addCustomGameFromWindow(win));
    root.appendChild(row);
  }
}

async function refreshGameWindows() {
  $("error").textContent = "";
  $("game-window-list").replaceChildren();
  const loading = document.createElement("div");
  loading.className = "hint";
  loading.textContent = "scanning running windows…";
  $("game-window-list").appendChild(loading);
  try {
    gameWindows = await invoke("list_game_windows");
    renderGameWindows();
  } catch (e) {
    $("error").textContent = e;
    gameWindows = [];
    renderGameWindows();
  }
}

async function showGameWindowPicker() {
  $("game-window-picker").hidden = false;
  await refreshGameWindows();
}

function hideGameWindowPicker() {
  $("game-window-picker").hidden = true;
}

function addCustomGameFromWindow(win) {
  const name = gameNameFromWindow(win);
  customGames.push(normalizeCustomGame({
    id: customGameId(name),
    name,
    enabled: true,
    exe_name: win.exe_name || "",
    process_path: win.exe_path || null,
    window_title: win.title || "",
    recording_mode: "replays_only",
  }));
  hideGameWindowPicker();
  renderCustomGames();
  $("settings-status").textContent = "custom game added - save to apply";
}

function updateGameDetectionStatus() {
  if (activeDetectedGame && activeDetectedGame.active) {
    $("game-detection-status").textContent =
      `Active: ${activeDetectedGame.name} · ${activeDetectedGame.window_title}`;
  } else {
    $("game-detection-status").textContent = customGames.length
      ? "No saved custom game is active."
      : "Add a running game window, then save.";
  }
}

function updateRegionFields() {
  $("set-region-width").value = regionState.width;
  $("set-region-height").value = regionState.height;
  $("set-region-x").value = regionState.x;
  $("set-region-y").value = regionState.y;
  const display = activeDisplay();
  $("region-display-label").textContent = display
    ? `${display.name} · ${display.width}x${display.height} at ${display.x}, ${display.y}`
    : "no displays";
  $("region-size-label").textContent = `${regionState.width}x${regionState.height}`;
}

function renderDisplayMenu() {
  const menu = $("region-display-menu");
  menu.replaceChildren();
  for (const display of displays) {
    const item = document.createElement("button");
    item.type = "button";
    item.textContent = display.name + (display.is_primary ? " (primary)" : "");
    item.addEventListener("click", () => {
      hideRegionMenu();
      setRegion(regionForDisplay(display));
    });
    menu.appendChild(item);
  }
}

function renderRegionEditor() {
  const editor = $("capture-region-editor");
  if (editor.hidden) return;
  const map = $("display-map");
  const inner = $("display-map-inner");
  const box = $("region-box");
  inner.querySelectorAll(".display-tile").forEach((node) => node.remove());
  if (!displays.length) {
    updateRegionFields();
    box.hidden = true;
    return;
  }
  const display = activeDisplay();
  if (display) {
    regionState = clampRegionToDisplay(regionState, display);
  }
  const mapWidth = Math.max(320, map.clientWidth);
  const mapHeight = displayMapHeight(displays, mapWidth, 10);
  map.style.height = `${mapHeight}px`;
  regionLayout = displayMapLayout(displays, mapWidth, mapHeight, 10);
  inner.style.width = "100%";
  inner.style.height = "100%";

  for (const item of regionLayout.displays) {
    const displayInfo = displays.find((d) => d.id === item.id);
    const tile = document.createElement("button");
    tile.type = "button";
    tile.className =
      "display-tile" +
      (displayInfo && displayInfo.is_primary ? " primary" : "") +
      (displayInfo && displayInfo.id === regionState.display_id ? " active" : "");
    tile.style.left = `${item.left}px`;
    tile.style.top = `${item.top}px`;
    tile.style.width = `${item.width}px`;
    tile.style.height = `${item.height}px`;
    tile.addEventListener("click", () => {
      if (displayInfo) setRegion({ ...regionState, display_id: displayInfo.id });
    });
    tile.addEventListener("contextmenu", (ev) => showRegionMenu(ev, displayInfo && displayInfo.id));
    const label = document.createElement("span");
    label.textContent = displayInfo ? displayInfo.name : item.id;
    tile.appendChild(label);
    inner.insertBefore(tile, box);
  }

  const bounds = regionLayout.bounds;
  const scale = regionLayout.scale;
  box.hidden = false;
  box.style.left = `${10 + (regionState.x - bounds.x) * scale}px`;
  box.style.top = `${10 + (regionState.y - bounds.y) * scale}px`;
  box.style.width = `${regionState.width * scale}px`;
  box.style.height = `${regionState.height * scale}px`;
  updateRegionFields();
  renderDisplayMenu();
}

function regionFromFields() {
  return {
    display_id: regionState.display_id,
    x: Number($("set-region-x").value),
    y: Number($("set-region-y").value),
    width: Number($("set-region-width").value),
    height: Number($("set-region-height").value),
  };
}

function startRegionDrag(kind, ev) {
  if (!regionLayout || !activeDisplay()) return;
  regionDrag = {
    kind,
    startX: ev.clientX,
    startY: ev.clientY,
    region: { ...regionState },
  };
  $("region-box").setPointerCapture(ev.pointerId);
  ev.preventDefault();
  ev.stopPropagation();
}

function moveRegionDrag(ev) {
  if (!regionDrag || !regionLayout) return;
  const dx = Math.round((ev.clientX - regionDrag.startX) / regionLayout.scale);
  const dy = Math.round((ev.clientY - regionDrag.startY) / regionLayout.scale);
  const base = regionDrag.region;
  if (regionDrag.kind === "resize") {
    setRegion({
      ...base,
      width: base.width + dx,
      height: base.height + dy,
    });
  } else {
    setRegion({
      ...base,
      x: base.x + dx,
      y: base.y + dy,
    });
  }
}

function endRegionDrag() {
  regionDrag = null;
}

function showRegionMenu(ev, displayId = null) {
  ev.preventDefault();
  ev.stopPropagation();
  regionMenuDisplayId = displayId || (activeDisplay() && activeDisplay().id);
  renderDisplayMenu();
  const menu = $("capture-region-menu");
  menu.hidden = false;
  menu.style.left = `${ev.clientX}px`;
  menu.style.top = `${ev.clientY}px`;
}

function hideRegionMenu() {
  $("capture-region-menu").hidden = true;
  regionMenuDisplayId = null;
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

async function refreshMemoryUsage() {
  try {
    const s = await invoke("memory_status");
    $("memory-usage").textContent = `Using ${fmtBytes(s.private_working_set_bytes)} RAM`;
  } catch (_) {
    $("memory-usage").textContent = "Using -- RAM";
  }
}

async function refreshClips() {
  clipsCache = await invoke("list_clips");
  renderClips();
  if (currentClip) {
    const fresh = clipsCache.find((clip) => clip.path === currentClip.path);
    if (fresh) currentClip = fresh;
    else closeReview();
  }
}

// Leading icon per clip kind. Static markup (no clip data) — innerHTML is safe.
const CLIP_KIND_ICONS = {
  replay:
    '<svg viewBox="0 0 24 24"><path d="M7 2v11h3v9l7-12h-4l4-8z"/></svg>',
  trim:
    '<svg viewBox="0 0 24 24"><path d="M9.64 7.64c.23-.5.36-1.05.36-1.64 0-2.21-1.79-4-4-4S2 3.79 2 6s1.79 4 4 4c.59 0 1.14-.13 1.64-.36L10 12l-2.36 2.36C7.14 14.13 6.59 14 6 14c-2.21 0-4 1.79-4 4s1.79 4 4 4 4-1.79 4-4c0-.59-.13-1.14-.36-1.64L12 14l7 7h3v-1L9.64 7.64zM6 8c-1.1 0-2-.89-2-2s.9-2 2-2 2 .89 2 2-.9 2-2 2zm0 12c-1.1 0-2-.89-2-2s.9-2 2-2 2 .89 2 2-.9 2-2 2zm6-7.5c-.28 0-.5-.22-.5-.5s.22-.5.5-.5.5.22.5.5-.22.5-.5.5zM19 3l-6 6 2 2 7-7V3z"/></svg>',
};
const CLIP_KIND_LABELS = { replay: "Buffered replay", trim: "Trimmed export" };

// Clip names come from disk; build rows with textContent, never innerHTML.
function clipRow(c) {
  const el = document.createElement("div");
  el.className = "clip" + (currentClip && currentClip.path === c.path ? " active" : "");
  el.title = c.name;

  const kind = clipKind(c.name);
  const icon = document.createElement("div");
  icon.className = "clip-kind " + kind;
  icon.title = CLIP_KIND_LABELS[kind];
  // Static per-kind markup, no clip data — innerHTML is safe here.
  icon.innerHTML = CLIP_KIND_ICONS[kind];

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

  el.append(icon, meta, del);
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
  updateStageFrame();
  video.src = convertFileSrc(clip.path);
  video.playbackRate = Number($("rate-select").value);
  setTrim(0, clip.duration_s ?? (clip.markers ? clip.markers.duration_s : 0));
  renderMarkers();
  renderRuler();
  renderClips();
  paintTimeline();
  noteActivity();
  requestAnimationFrame(updateStageFrame);
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

function renderVisibleSettingsSection() {
  const active = document.querySelector("#settings-tabs .tab.active");
  if (settingsOpen && active && active.dataset.tab === "capture") {
    requestAnimationFrame(renderRegionEditor);
  }
  if (settingsOpen && active && active.dataset.tab === "games") {
    renderCustomGames();
    updateGameDetectionStatus();
  }
}

function toggleSettings(open = !settingsOpen) {
  const wasOpen = settingsOpen;
  settingsOpen = open;
  // The clip survives the round-trip; just don't play behind the page.
  if (settingsOpen && !video.paused) video.pause();
  if (settingsOpen && !wasOpen) {
    loadAudioDevices();
    loadVideoEncoders();
  }
  // Closing discards unsaved edits by repainting from last-saved settings.
  if (wasOpen && !settingsOpen && currentSettings) fillSettings(currentSettings);
  updateViews();
  renderVisibleSettingsSection();
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
  requestAnimationFrame(updateStageFrame);
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
  syncRangeProgress($("volume-slider"));
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

async function chooseMediaFolder() {
  try {
    const selected = await invoke("choose_media_folder", {
      current: $("set-media-dir").value,
    });
    if (selected) {
      $("set-media-dir").value = selected;
      $("settings-status").textContent = "folder selected - save to apply";
    }
  } catch (e) {
    $("error").textContent = e;
  }
}

async function chooseReplayCacheFolder() {
  try {
    const selected = await invoke("choose_replay_cache_folder", {
      current: $("set-replay-disk-dir").value,
    });
    if (selected) {
      $("set-replay-disk-dir").value = selected;
      $("settings-status").textContent = "replay cache folder selected - save to apply";
    }
  } catch (e) {
    $("error").textContent = e;
  }
}

/* ---- backend events ---- */

listen("status", (e) => {
  const s = e.payload;
  recordingActive = s.recording;
  $("dot").className = "dot" + (s.recording ? " on" : "");
  $("rail-dot").className = "dot" + (s.recording ? " on" : "");
  updateCaptureStatus();
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

listen("mic-test", (e) => {
  if (!micTestRunning) return;
  const result = e.payload || {};
  playMicSamples(result.samples || []);
  const level = micMeterLevel(result);
  const peakPct = Math.round(Math.max(0, Math.min(1, Number(result.peak) || 0)) * 100);
  if (!result.sample_count) {
    setMicTestStatus("no input", 0);
  } else if (peakPct <= 1) {
    setMicTestStatus("quiet", level);
  } else {
    setMicTestStatus(`${peakPct}%`, level);
  }
});

listen("mic-test-error", (e) => {
  stopMicTestUi("error");
  $("error").textContent = e.payload;
});

listen("mic-test-stopped", () => {
  if (micTestRunning) stopMicTestUi("stopped");
});

listen("game-detection", (e) => {
  activeDetectedGame = e.payload || null;
  updateCaptureStatus();
  updateGameDetectionStatus();
});

/* ---- wiring ---- */

$("save").addEventListener("click", () => invoke("save_replay"));
$("capture-status").addEventListener("click", toggleRecording);
$("rail-status").addEventListener("click", toggleRecording);
$("set-capture").addEventListener("change", syncCaptureFields);
for (const id of ["set-output-enabled", "set-mic-enabled"]) {
  $(id).addEventListener("change", syncAudioFields);
}
for (const id of ["set-output-volume", "set-mic-volume"]) {
  $(id).addEventListener("input", () => {
    syncRangeProgress($(id));
    syncAudioFields();
  });
  $(id).addEventListener("change", () => {
    syncRangeProgress($(id));
    syncAudioFields();
  });
}
$("test-mic").addEventListener("click", testMic);
$("add-custom-game").addEventListener("click", showGameWindowPicker);
$("refresh-game-windows").addEventListener("click", refreshGameWindows);
$("cancel-game-picker").addEventListener("click", hideGameWindowPicker);
$("choose-media-folder").addEventListener("click", chooseMediaFolder);
$("choose-replay-cache-folder").addEventListener("click", chooseReplayCacheFolder);
$("set-replay-disk-enabled").addEventListener("change", syncReplayStorageFields);
$("set-replay-disk-quota").addEventListener("input", syncReplayStorageFields);
$("set-replay-disk-quota").addEventListener("change", syncReplayStorageFields);
for (const id of ["set-buffer", "set-replay", "set-encoder", "set-bitrate", "set-fps"]) {
  $(id).addEventListener("input", syncRecordingFields);
  $(id).addEventListener("change", syncRecordingFields);
}
document.querySelectorAll("[data-replay-preset]").forEach((button) => {
  button.addEventListener("click", () => {
    $("set-replay").value = button.dataset.replayPreset;
    syncRangeProgress($("set-replay"));
    syncRecordingFields();
  });
});
for (const id of ["set-region-width", "set-region-height", "set-region-x", "set-region-y"]) {
  $(id).addEventListener("change", () => setRegion(regionFromFields()));
  $(id).addEventListener("blur", () => setRegion(regionFromFields()));
}
$("display-map").addEventListener("contextmenu", showRegionMenu);
$("region-box").addEventListener("pointerdown", (ev) => {
  startRegionDrag(ev.target.dataset.regionResize ? "resize" : "move", ev);
});
$("region-box").addEventListener("pointermove", moveRegionDrag);
$("region-box").addEventListener("pointerup", endRegionDrag);
$("region-box").addEventListener("pointercancel", endRegionDrag);
$("region-box").addEventListener("lostpointercapture", endRegionDrag);
document.querySelectorAll("#region-align-menu button").forEach((button) => {
  button.addEventListener("click", () => {
    const display = menuDisplay();
    if (display) setRegion(alignRegion(regionState, display, button.dataset.align));
    hideRegionMenu();
  });
});
document.addEventListener("click", (ev) => {
  if (!$("capture-region-menu").contains(ev.target)) hideRegionMenu();
});
window.addEventListener("resize", () => {
  renderRegionEditor();
  updateStageFrame();
});
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
  updateStageFrame();
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
  syncRangeProgress($("volume-slider"));
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
$("set-hotkey").addEventListener("focus", beginHotkeyCapture);
$("set-hotkey").addEventListener("click", beginHotkeyCapture);
$("set-hotkey").addEventListener("keydown", recordHotkey);
$("set-hotkey").addEventListener("paste", (ev) => ev.preventDefault());
$("set-hotkey").addEventListener("blur", () => {
  if (hotkeyCaptureActive) endHotkeyCapture("Shortcut unchanged.");
});

document.querySelectorAll("#settings-tabs .tab").forEach((tab) => {
  tab.addEventListener("click", () => {
    document
      .querySelectorAll("#settings-tabs .tab")
      .forEach((t) => t.classList.toggle("active", t === tab));
    document.querySelectorAll(".settings-section").forEach((s) => {
      s.hidden = s.dataset.section !== tab.dataset.tab;
    });
    renderVisibleSettingsSection();
  });
});

$("timeline").addEventListener("pointerdown", (ev) => {
  if (ev.target === $("handle-in")) startDrag("in", ev);
  else if (ev.target === $("handle-out")) startDrag("out", ev);
  else startDrag("scrub", ev);
});
$("timeline").addEventListener("pointermove", moveDrag);

stage.addEventListener("pointermove", noteActivity);
stage.addEventListener("pointerdown", noteActivity);
stage.addEventListener("pointerleave", () => {
  // Leaving the stage while playing hides the bar immediately.
  lastActivityMs = -Infinity;
  updateOverlay();
});
new ResizeObserver(updateStageFrame).observe(stage);
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
syncAllRangeProgress();
invoke("get_settings").then(fillSettings).catch((e) => $("error").textContent = e);
loadDisplays();
loadAudioDevices();
loadVideoEncoders();
refresh();
refreshMemoryUsage();
setInterval(refreshMemoryUsage, 2000);
