// DOM wiring + Tauri bridge. All player math and formatting lives in
// player-core.js (PlayerCore), which is unit-tested from Rust — keep this
// file to event plumbing and rendering.
const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const appWindow = window.__TAURI__.window.getCurrentWindow();
const $ = (id) => document.getElementById(id);

// Custom window chrome — the native title bar is disabled (decorations: false).
$("win-min").addEventListener("click", async () => {
  try {
    await invoke("minimize_main_window");
  } catch (e) {
    $("error").textContent = e;
  }
});
$("win-max").addEventListener("click", () => appWindow.toggleMaximize());
$("win-close").addEventListener("click", requestWindowClose);
const {
  fmtBytes,
  fmtDur,
  fmtTenths,
  fmtAgo,
  overlayVisible,
  clampTime,
  percentFor,
  percentForView,
  timelineTimeView,
  clampView,
  zoomView,
  panView,
  setViewEdge,
  viewForRange,
  followView,
  snapTime,
  snapCandidates,
  frameStep,
  editPoints,
  MIN_VIEW_SPAN_S,
  DEFAULT_FOLLOW_MODE,
  SNAP_THRESHOLD_PX,
  DEFAULT_FINE_STEP_S,
  resolveTrim,
  trimDrag,
  slideTrim,
  trimSummary,
  nextMarker,
  prevMarker,
  markerSummary,
  markerStyle,
  markerDigest,
  playerSummaryLabel,
  rulerMarksRange,
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
  outputResolutionOption,
  captureSourceLabel,
  captureStatusLabel,
} = PlayerCore;

const video = $("video");
const stage = document.querySelector(".stage");
const stageFrame = $("stage-frame");
let currentClip = null;
let clipsCache = [];
let currentSettings = null;
let recordingActive = false;
let fullSessionRecordingActive = false;
let windowFocused = document.hasFocus();
let previewRequested = false;
let previewHasFrame = false;
let previewWindowMovePaused = false;
let previewWindowMoveTimer = 0;
let previewWindowMoveStart = null;
let displays = [];
let audioDevices = { outputs: [], inputs: [] };
let videoEncoders = [];
let gamePlugins = [];
let gamePluginSettings = {};
let customGames = [];
let gameWindows = [];
let activeDetectedGame = null;
let captureTargetDirty = false;
// Codecs WebView2 can decode in the review player (H.264 always; HEVC/AV1
// probed at startup). Drives the playback caveat and the recorder's
// Automatic-codec policy via report_decode_support.
let decodableCodecs = ["h264"];
let regionState = { display_id: null, x: 0, y: 0, width: 1920, height: 1080 };
let regionLayout = null;
let regionDrag = null;
let regionMenuDisplayId = null;
let micTestRunning = false;
let micAudioContext = null;
let micAudioCursor = 0;
let micAudioSources = [];
let pendingUpdate = null;
let updateCheckRunning = false;
let hotkeyCaptureActive = false;
let trimStart = 0;
let trimEnd = 0;
let dragging = null;
let rafId = 0;
// Timeline zoom: the visible window into the clip. zoomSpan === 0 means fully
// zoomed out (the whole clip is shown); a smaller span shows [zoomStart, +span].
let zoomStart = 0;
let zoomSpan = 0;
// Active navigator drag: { mode:"pan"|"left"|"right", grab?, pointerId } or null.
let overviewDrag = null;
// Magnetic snapping for scrub/trim drags (toggle with S, hold Alt to bypass).
let snapEnabled = true;
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
  const cloud = { ...defaultCloudSettings(), ...(s.cloud || {}) };
  cloud.uploads = { ...(cloud.uploads || {}) };
  gamePluginSettings = normalizeGamePluginSettingsMap(games.plugins || {});
  customGames = (games.custom_games || []).map(normalizeCustomGame);
  currentSettings = {
    ...s,
    audio,
    replay_storage: replayStorage,
    cloud,
    games: {
      ...games,
      plugins: { ...gamePluginSettings },
      custom_games: customGames.map((game) => ({ ...game })),
    },
  };
  regionState = s.capture_region ?? regionState;
  captureTargetDirty = false;
  renderCaptureTargetSelect();
  $("set-games-auto-detect").checked = !!games.auto_detect;
  $("set-output-enabled").checked = !!audio.output_enabled;
  $("set-output-volume").value = String(Number.isFinite(audio.output_volume) ? audio.output_volume : 1);
  $("set-mic-enabled").checked = !!audio.mic_enabled;
  $("set-mic-volume").value = String(Number.isFinite(audio.mic_volume) ? audio.mic_volume : 1);
  $("set-mic-mono").checked = (audio.mic_channels || "mono") === "mono";
  $("set-buffer").value = Number(s.buffer_seconds) || ((Number(s.replay_window_s) || 60) + 15);
  $("set-replay").value = Math.min(120, Number(s.replay_window_s) || 60);
  $("set-encoder").value = s.video_encoder || "auto";
  $("set-output-resolution").value = outputResolutionOption(s.output_resolution).id;
  $("set-bitrate").value = s.video_quality
    ? PlayerCore.qualityIndexForId(s.video_quality)
    : qualityIndexForBitrate(s.bitrate_mbps, $("set-output-resolution").value);
  $("set-fps").value = smoothnessIndexForFps(s.fps);
  $("set-quota").value = s.disk_quota_gb;
  $("set-media-dir").value = s.media_dir ?? "";
  $("set-replay-disk-enabled").checked = replayStorage.mode === "disk";
  $("set-replay-disk-dir").value = replayStorage.disk_dir || "";
  $("set-replay-disk-quota").value = replayStorage.disk_quota_gb ?? 2;
  $("set-replay-disk-ack").checked = !!replayStorage.disk_acknowledged;
  $("set-hotkey").value = s.hotkey;
  $("save-hotkey").textContent = s.hotkey;
  $("set-open-on-startup").checked = !!s.open_on_startup;
  $("set-close-to-tray").checked = s.close_to_tray !== false;
  $("set-minimize-to-tray").checked = !!s.minimize_to_tray;
  $("set-capture-preview-enabled").checked = s.capture_preview_enabled !== false;
  $("set-update-channel").value = s.update_channel || "nightly";
  fillCloudSettings(cloud);
  endHotkeyCapture("Click the field to record a new shortcut.");
  syncCaptureFields();
  renderAudioDeviceSelects();
  renderVideoEncoderSelect();
  syncAudioFields();
  syncRecordingFields();
  syncReplayStorageFields();
  renderGamePlugins();
  renderCustomGames();
  updateGameDetectionStatus();
  updateCaptureStatus();
}

function readSettings() {
  const replay = Number($("set-replay").value);
  const capture = selectedCaptureSettings();
  const preserveLegacyWindow =
    !captureTargetDirty
    && currentSettings
    && currentSettings.capture_mode === "window_title"
    && String(currentSettings.window_title || "").trim().length > 0;
  return {
    capture_mode: preserveLegacyWindow ? "window_title" : capture.capture_mode,
    window_title: preserveLegacyWindow ? currentSettings.window_title : "",
    capture_region: preserveLegacyWindow
      ? (currentSettings.capture_region || capture.capture_region)
      : capture.capture_region,
    games: {
      auto_detect: $("set-games-auto-detect").checked,
      plugins: readGamePluginSettings(),
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
    output_resolution: outputResolutionOption($("set-output-resolution").value).id,
    video_quality: recordingQualityPreset(Number($("set-bitrate").value)).id,
    bitrate_mbps: recordingQualityPreset(
      Number($("set-bitrate").value),
      $("set-output-resolution").value
    ).bitrate,
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
    open_on_startup: $("set-open-on-startup").checked,
    close_to_tray: $("set-close-to-tray").checked,
    minimize_to_tray: $("set-minimize-to-tray").checked,
    capture_preview_enabled: $("set-capture-preview-enabled").checked,
    update_channel: $("set-update-channel").value,
    cloud: readCloudSettings(),
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
    plugins: {},
    custom_games: [],
  };
}

function defaultCloudSettings() {
  return {
    host_url: "",
    public_url: null,
    connected_user_id: null,
    connected_username: null,
    credential_target: null,
    default_visibility: "private",
    delete_local_after_upload: false,
    auto_upload_rules: false,
    uploads: {},
  };
}

function fillCloudSettings(cloud) {
  $("cloud-host-url").value = cloud.host_url || "";
  $("cloud-username").value = cloud.connected_username || "";
  $("cloud-password").value = "";
  $("cloud-default-visibility").value = cloud.default_visibility || "private";
  $("cloud-delete-local-after-upload").checked = !!cloud.delete_local_after_upload;
  $("cloud-auto-upload-rules").checked = false;
  $("cloud-http-confirm").checked = false;
  const connected = cloud.connected_user_id && cloud.credential_target;
  $("cloud-connection-status").textContent = connected
    ? `Connected as ${cloud.connected_username || cloud.connected_user_id}`
    : "Not connected";
  $("cloud-disconnect").disabled = !connected;
  $("cloud-connect-status").textContent = "";
}

function readCloudSettings() {
  const existing = currentSettings && currentSettings.cloud
    ? currentSettings.cloud
    : defaultCloudSettings();
  return {
    ...existing,
    default_visibility: $("cloud-default-visibility").value || "private",
    delete_local_after_upload: $("cloud-delete-local-after-upload").checked,
    auto_upload_rules: false,
    uploads: { ...(existing.uploads || {}) },
  };
}

function defaultGamePluginSettings(plugin) {
  return {
    enabled: plugin ? plugin.default_enabled !== false : true,
    recording_mode: normalizeGameRecordingMode(
      plugin && plugin.default_recording_mode ? plugin.default_recording_mode : "full_session"
    ),
  };
}

function normalizeGameRecordingMode(mode) {
  return mode === "full_session" ? "full_session" : "replays_only";
}

function normalizeGamePluginId(raw) {
  return String(raw || "")
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
}

function normalizeGamePluginSettings(settings, plugin = null) {
  const defaults = defaultGamePluginSettings(plugin);
  return {
    enabled: settings && Object.prototype.hasOwnProperty.call(settings, "enabled")
      ? settings.enabled !== false
      : defaults.enabled,
    recording_mode: normalizeGameRecordingMode(
      settings && settings.recording_mode ? settings.recording_mode : defaults.recording_mode
    ),
  };
}

function normalizeGamePluginSettingsMap(settings) {
  const out = {};
  for (const [id, value] of Object.entries(settings || {})) {
    const cleanId = normalizeGamePluginId(id);
    if (cleanId) out[cleanId] = normalizeGamePluginSettings(value);
  }
  return out;
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
    icon: game.icon ? String(game.icon) : null,
  };
}

function selectedRecordingMode(name, fallback = "replays_only") {
  const input = document.querySelector(`input[name="${name}"]:checked`);
  return input ? normalizeGameRecordingMode(input.value) : normalizeGameRecordingMode(fallback);
}

function setRecordingMode(name, mode) {
  const normalized = normalizeGameRecordingMode(mode);
  document.querySelectorAll(`input[name="${name}"]`).forEach((input) => {
    input.checked = input.value === normalized;
  });
}

function gamePluginSetting(plugin) {
  return normalizeGamePluginSettings(gamePluginSettings[plugin.id], plugin);
}

function readGamePluginSettings() {
  const next = {
    ...normalizeGamePluginSettingsMap(
      currentSettings && currentSettings.games ? currentSettings.games.plugins : {}
    ),
  };
  for (const plugin of gamePlugins) {
    const existing = gamePluginSetting(plugin);
    const checkbox = document.querySelector(`[data-game-plugin-enabled="${plugin.id}"]`);
    next[plugin.id] = normalizeGamePluginSettings({
      enabled: checkbox ? checkbox.checked : existing.enabled,
      recording_mode: selectedRecordingMode(
        `game-plugin-mode-${plugin.id}`,
        existing.recording_mode
      ),
    }, plugin);
  }
  gamePluginSettings = next;
  return { ...gamePluginSettings };
}

function gamePluginSummary(plugin, settings = gamePluginSetting(plugin)) {
  if (!settings.enabled) {
    return `Disabled. ${plugin.name} will not change capture or start session recordings.`;
  }
  if (settings.recording_mode === "full_session") {
    return "Full-session recording starts when the match window appears. Takes priority over matching custom games.";
  }
  return "Replay capture switches to the match window without saving a full session. Takes priority over matching custom games.";
}

function updateGamePluginSummary(plugin) {
  const summary = document.querySelector(`[data-game-plugin-summary="${plugin.id}"]`);
  if (summary) summary.textContent = gamePluginSummary(plugin);
}

function renderGamePluginModeControl(plugin, settings) {
  const control = document.createElement("div");
  control.className = "segmented-control game-profile-mode";
  control.setAttribute("role", "radiogroup");
  control.setAttribute("aria-label", `${plugin.name} recording mode`);
  [
    ["replays_only", "Replays only"],
    ["full_session", "Full session"],
  ].forEach(([value, label]) => {
    const option = document.createElement("label");
    const input = document.createElement("input");
    input.type = "radio";
    input.name = `game-plugin-mode-${plugin.id}`;
    input.value = value;
    input.checked = settings.recording_mode === value;
    input.addEventListener("change", () => {
      if (input.checked) {
        gamePluginSettings[plugin.id] = {
          ...gamePluginSetting(plugin),
          recording_mode: value,
        };
        updateGamePluginSummary(plugin);
        updateGameDetectionStatus();
      }
    });
    const text = document.createElement("span");
    text.textContent = label;
    option.append(input, text);
    control.appendChild(option);
  });
  return control;
}

function renderGamePlugins() {
  const root = $("supported-games");
  root.replaceChildren();
  if (!gamePlugins.length) {
    const empty = document.createElement("div");
    empty.className = "hint";
    empty.textContent = "no game plugins installed";
    root.appendChild(empty);
    return;
  }

  for (const plugin of gamePlugins) {
    const settings = gamePluginSetting(plugin);
    gamePluginSettings[plugin.id] = settings;

    const row = document.createElement("div");
    row.className = "game-profile supported";
    row.dataset.gamePluginId = plugin.id;

    const enabled = document.createElement("label");
    enabled.className = "check-line";
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = settings.enabled;
    checkbox.dataset.gamePluginEnabled = plugin.id;
    checkbox.addEventListener("change", () => {
      gamePluginSettings[plugin.id] = {
        ...gamePluginSetting(plugin),
        enabled: checkbox.checked,
      };
      updateGamePluginSummary(plugin);
      updateGameDetectionStatus();
    });
    enabled.appendChild(checkbox);

    const icon = gameIconEl(plugin.icon, plugin.name);

    const meta = document.createElement("div");
    meta.className = "game-profile-meta";
    const name = document.createElement("strong");
    name.textContent = plugin.name;
    const summary = document.createElement("span");
    summary.dataset.gamePluginSummary = plugin.id;
    summary.textContent = gamePluginSummary(plugin, settings);
    meta.append(name, summary);

    row.append(enabled, icon, meta, renderGamePluginModeControl(plugin, settings));
    root.appendChild(row);
  }
}

function displayCaptureValue(display) {
  return `display:${display.id}`;
}

function displayForCaptureValue(value) {
  if (!String(value || "").startsWith("display:")) return null;
  const id = String(value).slice("display:".length);
  return displays.find((display) => display.id === id) || null;
}

function isFullDisplayRegion(region, display) {
  return !!region && !!display
    && region.display_id === display.id
    && Number(region.x) === display.x
    && Number(region.y) === display.y
    && Number(region.width) === display.width
    && Number(region.height) === display.height;
}

function captureSettingsValue(settings = currentSettings) {
  if (settings && settings.capture_mode === "display_region") {
    const display = displays.find((item) => isFullDisplayRegion(settings.capture_region, item));
    return display ? displayCaptureValue(display) : "display_region";
  }
  const display = primaryDisplay();
  return display ? displayCaptureValue(display) : "primary_monitor";
}

function displayLabel(display) {
  const primary = display.is_primary ? " (primary)" : "";
  return `${display.name}${primary} - ${display.width}x${display.height}`;
}

function renderCaptureTargetSelect() {
  const select = $("set-capture");
  const desired = captureSettingsValue();
  select.replaceChildren();
  if (displays.length) {
    for (const display of displays) {
      const option = document.createElement("option");
      option.value = displayCaptureValue(display);
      option.textContent = displayLabel(display);
      select.appendChild(option);
    }
  } else {
    const option = document.createElement("option");
    option.value = "primary_monitor";
    option.textContent = "Primary display";
    select.appendChild(option);
  }
  const region = document.createElement("option");
  region.value = "display_region";
  region.textContent = "SET REGION";
  select.appendChild(region);
  select.value = Array.from(select.options).some((option) => option.value === desired)
    ? desired
    : captureSettingsValue({ capture_mode: "primary_monitor" });
  syncCaptureFields();
}

function selectedCaptureSettings() {
  const display = displayForCaptureValue($("set-capture").value);
  if (display) {
    return {
      capture_mode: "display_region",
      capture_region: regionForDisplay(display),
    };
  }
  return {
    capture_mode: $("set-capture").value === "display_region" ? "display_region" : "primary_monitor",
    capture_region: regionState,
  };
}

function syncCaptureFields() {
  const display = displayForCaptureValue($("set-capture").value);
  if (display) {
    regionState = regionForDisplay(display);
  }
  const isEditableRegion = $("set-capture").value === "display_region";
  $("capture-region-editor").hidden = !isEditableRegion;
  if (isEditableRegion) renderRegionEditor();
  updateCaptureStatus();
}

function syncRecordingFields() {
  const replay = Number($("set-replay").value);
  const encoder = selectedVideoEncoder();
  const outputResolution = outputResolutionOption($("set-output-resolution").value);
  const quality = recordingQualityPreset(Number($("set-bitrate").value), outputResolution.id);
  const smoothness = smoothnessPreset(Number($("set-fps").value));
  syncRangeProgress($("set-replay"));
  syncRangeProgress($("set-bitrate"));
  syncRangeProgress($("set-fps"));
  $("replay-summary").textContent = `Save Replay writes the last ${settingDurationLabel(replay)}.`;
  $("replay-summary").className = "setting-summary";
  const encoderSummary = $("encoder-summary");
  if (encoder.id === "auto") {
    encoderSummary.textContent =
      "Clipline records H.264 when available for broad playback compatibility.";
    encoderSummary.classList.remove("warn");
  } else {
    const caveat = PlayerCore.encoderCodecCaveat(encoder.codec, decodableCodecs);
    encoderSummary.textContent = caveat || `${encoder.name} is used for new recordings.`;
    encoderSummary.classList.toggle("warn", Boolean(caveat));
  }
  $("output-resolution-summary").textContent =
    outputResolution.id === "source"
      ? "Uses the captured size, capped only when needed for encoder compatibility."
      : `${outputResolution.label} output, ${outputResolution.hint}.`;
  $("quality-summary").textContent = `${quality.label} quality - ${quality.hint}.`;
  $("fps-summary").textContent = `${smoothness.label} - ${smoothness.hint}.`;
  syncReplayStorageFields();
}

function syncReplayStorageFields() {
  const enabled = $("set-replay-disk-enabled").checked;
  const fields = $("replay-disk-fields");
  fields.hidden = !enabled;
  const quality = recordingQualityPreset(Number($("set-bitrate").value), $("set-output-resolution").value);
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
    const caveat = PlayerCore.encoderCodecCaveat(encoder.codec, decodableCodecs);
    opt.textContent = caveat ? `${encoder.name} (limited playback)` : encoder.name;
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
      : fallbackCaptureSourceLabel(currentSettings || { capture_mode: "primary_monitor" });
  $("capture-status-label").textContent = captureStatusLabel(
    source,
    recordingActive,
    fullSessionRecordingActive
  );
  $("capture-status").classList.toggle("stopped", !recordingActive);
  $("capture-status").setAttribute("aria-pressed", String(recordingActive));
  $("capture-status").title = recordingActive ? "Stop recording" : `Start ${source} recording`;
  $("rail-status").classList.toggle("stopped", !recordingActive);
  $("rail-status").setAttribute("aria-pressed", String(recordingActive));
  $("rail-status").title = $("capture-status").title;
  $("rail-status-text").textContent = recordingActive ? "Rec" : "Off";
  $("save").disabled = !recordingActive;
  $("rail-save").disabled = !recordingActive;
  updateCapturePreview();
}

function fallbackCaptureSourceLabel(settings) {
  if (settings && settings.capture_mode === "display_region") {
    const display = displays.find((item) => isFullDisplayRegion(settings.capture_region, item));
    if (display) return `Display: ${display.name}`;
  }
  return captureSourceLabel(settings);
}

function emptyPreviewVisible() {
  return !settingsOpen && !currentClip;
}

function previewShouldRun() {
  return (
    emptyPreviewVisible()
    && recordingActive
    && currentSettings?.capture_preview_enabled !== false
    && windowFocused
    && !previewWindowMovePaused
  );
}

function setPreviewOverlay(title, message, ready = false) {
  $("capture-preview-title").textContent = title;
  $("capture-preview-status").textContent = message;
  $("capture-preview-overlay").classList.toggle("ready", ready);
}

function resetCapturePreviewFrame() {
  previewHasFrame = false;
  const image = $("capture-preview-image");
  image.hidden = true;
  image.removeAttribute("src");
}

function updateCapturePreview() {
  const visible = emptyPreviewVisible();
  const shouldRun = previewShouldRun();
  const metaSource =
    activeDetectedGame && activeDetectedGame.active
      ? `Game: ${activeDetectedGame.name}`
      : fallbackCaptureSourceLabel(currentSettings || { capture_mode: "primary_monitor" });

  if (!shouldRun && previewHasFrame) resetCapturePreviewFrame();

  if (visible && currentSettings?.capture_preview_enabled === false) {
    setPreviewOverlay("Capture preview", "Display preview is off");
    $("capture-preview-meta").textContent = metaSource;
  } else if (!recordingActive) {
    setPreviewOverlay("Capture preview", "Recording is off");
    $("capture-preview-meta").textContent = "Start recording to show the active target";
  } else if (previewWindowMovePaused && visible) {
    setPreviewOverlay("Preview paused", "Moving window...");
    $("capture-preview-meta").textContent = metaSource;
  } else if (!windowFocused && visible) {
    setPreviewOverlay("Preview paused", "Focus Clipline to show preview");
    $("capture-preview-meta").textContent = metaSource;
  } else if (visible && previewHasFrame) {
    setPreviewOverlay("Capture preview", "", true);
    $("capture-preview-meta").textContent = metaSource;
  } else if (visible) {
    setPreviewOverlay("Capture preview", "Waiting for capture…");
    $("capture-preview-meta").textContent = metaSource;
  } else {
    setPreviewOverlay("Capture preview", "");
  }

  if (shouldRun === previewRequested) return;
  previewRequested = shouldRun;
  invoke("set_preview_active", { active: shouldRun }).then((activation) => {
    if (shouldRun && activation?.focused === false) {
      windowFocused = false;
      previewRequested = false;
      resetCapturePreviewFrame();
      updateCapturePreview();
      return;
    }
    if (shouldRun && activation?.enabled === false) {
      previewRequested = false;
      resetCapturePreviewFrame();
      updateCapturePreview();
      return;
    }
    if (shouldRun && !activation?.active) {
      previewRequested = false;
      setPreviewOverlay("Capture preview", "Preview unavailable");
    }
  }).catch((e) => {
    previewRequested = false;
    if (shouldRun) setPreviewOverlay("Capture preview", String(e));
  });
}

function setPreviewWindowMovePaused(paused) {
  if (previewWindowMovePaused === paused) return;
  previewWindowMovePaused = paused;
  if (!paused) previewRequested = false;
  updateCapturePreview();
}

function pausePreviewForWindowMove() {
  if (!emptyPreviewVisible()) return;
  clearTimeout(previewWindowMoveTimer);
  setPreviewWindowMovePaused(true);
  // Native window dragging can delay pointerup/timers until the move ends.
  // This fallback keeps preview from staying paused if the release event is
  // swallowed by the system drag.
  previewWindowMoveTimer = setTimeout(() => setPreviewWindowMovePaused(false), 900);
}

function resumePreviewAfterWindowMove() {
  previewWindowMoveStart = null;
  clearTimeout(previewWindowMoveTimer);
  previewWindowMoveTimer = setTimeout(() => setPreviewWindowMovePaused(false), 120);
}

function armPreviewWindowMovePause(ev) {
  clearTimeout(previewWindowMoveTimer);
  previewWindowMoveStart = { pointerId: ev.pointerId, x: ev.clientX, y: ev.clientY };
  previewWindowMoveTimer = setTimeout(() => { previewWindowMoveStart = null; }, 700);
}

function maybePausePreviewForWindowMove(ev) {
  if (!previewWindowMoveStart || ev.pointerId !== previewWindowMoveStart.pointerId) return;
  const dx = ev.clientX - previewWindowMoveStart.x;
  const dy = ev.clientY - previewWindowMoveStart.y;
  if ((dx * dx + dy * dy) < 16) return;
  previewWindowMoveStart = null;
  clearTimeout(previewWindowMoveTimer);
  pausePreviewForWindowMove();
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
    renderCaptureTargetSelect();
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

// Probe which codecs this WebView2 can actually decode and report them so
// Automatic recording never produces a clip the review player can't show.
function probeDecodableCodecs() {
  const probe = document.createElement("video");
  const supported = ["h264"];
  for (const { codec, mime } of PlayerCore.videoDecodeProbes()) {
    const verdict = probe.canPlayType(mime);
    if (verdict === "probably" || verdict === "maybe") supported.push(codec);
  }
  decodableCodecs = supported;
}

async function loadVideoEncoders() {
  probeDecodableCodecs();
  try {
    await invoke("report_decode_support", { codecs: decodableCodecs });
  } catch (e) {
    // Reporting is best-effort; the recorder defaults to H.264-safe Automatic.
  }
  try {
    videoEncoders = await invoke("probe_encoders");
  } catch (e) {
    videoEncoders = [];
    $("error").textContent = e;
  }
  renderVideoEncoderSelect();
  if (currentSettings) syncRecordingFields();
}

async function loadGamePlugins() {
  try {
    gamePlugins = await invoke("list_game_plugins");
    renderGamePlugins();
    updateGameDetectionStatus();
    // Clips may have rendered before plugins loaded; refresh their game icons.
    if (clipsCache.length) renderClips();
  } catch (e) {
    gamePlugins = [];
    $("error").textContent = e;
    renderGamePlugins();
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

    const icon = gameIconEl(game.icon, game.name);

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

    row.append(enabled, icon, meta, remove, gameRecordingModeControl(game, index));
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

async function addCustomGameFromWindow(win) {
  const name = gameNameFromWindow(win);
  // Pull the executable's icon now, while we still have its path. Best-effort:
  // a missing path or icon just leaves the game with the placeholder glyph.
  let icon = null;
  if (win.exe_path) {
    try {
      icon = await invoke("extract_window_icon", { exePath: win.exe_path });
    } catch (e) {
      icon = null;
    }
  }
  customGames.push(normalizeCustomGame({
    id: customGameId(name),
    name,
    enabled: true,
    exe_name: win.exe_name || "",
    process_path: win.exe_path || null,
    window_title: win.title || "",
    recording_mode: "replays_only",
    icon,
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
    if (!$("set-games-auto-detect").checked) {
      $("game-detection-status").textContent = "Game detection is off.";
      return;
    }
    const enabledPlugins = gamePlugins.filter((plugin) => gamePluginSetting(plugin).enabled);
    if (enabledPlugins.length) {
      const names = enabledPlugins.map((plugin) => plugin.name).join(", ");
      $("game-detection-status").textContent = `Waiting for: ${names}.`;
    } else if (customGames.length) {
      $("game-detection-status").textContent = "No saved custom game is active.";
    } else {
      $("game-detection-status").textContent = "Enable a game plugin or add a running game window, then save.";
    }
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
  $("rail-clips-count").textContent = compactCount(s.clip_count);
  $("rail-library-status").title = `${plural(s.clip_count, "clip")} in library`;
}

function compactCount(count) {
  return count > 99 ? "99+" : String(count);
}

function plural(count, singular) {
  return `${count} ${singular}${count === 1 ? "" : "s"}`;
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

function cloudSettings() {
  return currentSettings && currentSettings.cloud ? currentSettings.cloud : defaultCloudSettings();
}

function cloudConnected() {
  const cloud = cloudSettings();
  return Boolean(cloud.connected_user_id && cloud.credential_target);
}

function clipCloudRecord(clip) {
  const uploads = cloudSettings().uploads || {};
  return Object.values(uploads).find((record) => record && record.path === clip.path) || null;
}

function cloudStatusLabel(record) {
  if (!record) return "not uploaded";
  switch (record.upload_status) {
    case "queued": return "queued";
    case "uploading": return "uploading";
    case "processing": return "processing";
    case "uploaded_private": return "private";
    case "uploaded_public": return "public";
    case "failed": return "failed";
    case "retrying": return "retrying";
    default: return "not uploaded";
  }
}

function upsertCloudUploadRecord(record) {
  if (!record || !record.local_clip_id) return;
  const cloud = cloudSettings();
  cloud.uploads = { ...(cloud.uploads || {}), [record.local_clip_id]: record };
  if (currentSettings) currentSettings.cloud = cloud;
}

function upsertCloudProgress(progress) {
  if (!progress || !progress.local_clip_id) return;
  const current = (cloudSettings().uploads || {})[progress.local_clip_id] || {};
  upsertCloudUploadRecord({
    ...current,
    local_clip_id: progress.local_clip_id,
    path: progress.path || current.path || "",
    remote_clip_id: progress.remote_clip_id ?? current.remote_clip_id ?? null,
    remote_url: progress.remote_url ?? current.remote_url ?? null,
    visibility: current.visibility || cloudSettings().default_visibility || "private",
    upload_status: progress.upload_status || current.upload_status || "not_uploaded",
    error: progress.error ?? current.error ?? null,
    updated_at_unix: Math.floor(Date.now() / 1000),
  });
}

// Leading icon per clip kind. Static markup (no clip data) — innerHTML is safe.
const CLIP_KIND_ICONS = {
  replay:
    '<svg viewBox="0 0 24 24"><path d="M7 2v11h3v9l7-12h-4l4-8z"/></svg>',
  session:
    '<svg viewBox="0 0 24 24"><path d="M3 5h18v14H3V5zM5 6v2h2v-2zM9 6v2h2v-2zM13 6v2h2v-2zM17 6v2h2v-2zM5 16v2h2v-2zM9 16v2h2v-2zM13 16v2h2v-2zM17 16v2h2v-2z"/></svg>',
  trim:
    '<svg viewBox="0 0 24 24"><path d="M9.64 7.64c.23-.5.36-1.05.36-1.64 0-2.21-1.79-4-4-4S2 3.79 2 6s1.79 4 4 4c.59 0 1.14-.13 1.64-.36L10 12l-2.36 2.36C7.14 14.13 6.59 14 6 14c-2.21 0-4 1.79-4 4s1.79 4 4 4 4-1.79 4-4c0-.59-.13-1.14-.36-1.64L12 14l7 7h3v-1L9.64 7.64zM6 8c-1.1 0-2-.89-2-2s.9-2 2-2 2 .89 2 2-.9 2-2 2zm0 12c-1.1 0-2-.89-2-2s.9-2 2-2 2 .89 2 2-.9 2-2 2zm6-7.5c-.28 0-.5-.22-.5-.5s.22-.5.5-.5.5.22.5.5-.22.5-.5.5zM19 3l-6 6 2 2 7-7V3z"/></svg>',
};
const CLIP_KIND_LABELS = {
  replay: "Buffered replay",
  session: "Full session",
  trim: "Trimmed export",
};

// Neutral fallback when a game has no extractable/bundled icon. Static markup.
const GENERIC_GAME_ICON =
  '<svg viewBox="0 0 24 24"><path d="M3 5h18a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1h-7l1 2h2v2H6v-2h2l1-2H3a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1zm1 2v7h16V7H4z"/></svg>';

// A game-icon element: an <img> for a real icon (a plugin's bundled URL or an
// extracted data URL), falling back to a neutral glyph when absent or broken.
function gameIconEl(iconUrl, label) {
  if (iconUrl) {
    const img = document.createElement("img");
    img.className = "game-icon";
    img.src = iconUrl;
    img.alt = "";
    if (label) img.title = label;
    img.addEventListener("error", () => img.replaceWith(gamePlaceholderEl()));
    return img;
  }
  return gamePlaceholderEl();
}
function gamePlaceholderEl() {
  const el = document.createElement("div");
  el.className = "game-icon placeholder";
  el.innerHTML = GENERIC_GAME_ICON; // static markup, safe
  return el;
}

// Resolve a clip's recorded game to an icon, reusing the icons shown in
// settings: a plugin's bundled icon, or a custom game's extracted icon.
// Returns null for clips with no game, or a game no longer configured.
function clipGameIcon(clip) {
  const g = clip && clip.game;
  if (!g || !g.id) return null;
  const plugin = gamePlugins.find((p) => p.id === g.id);
  if (plugin && plugin.icon) return { url: plugin.icon, label: plugin.name };
  const custom = customGames.find((c) => c.id === g.id);
  if (custom && custom.icon) return { url: custom.icon, label: custom.name };
  return null;
}

// Clip names come from disk; build rows with textContent, never innerHTML.
function clipRow(c) {
  const el = document.createElement("div");
  el.className = "clip" + (currentClip && currentClip.path === c.path ? " active" : "");
  el.title = c.name;
  const cloudRecord = clipCloudRecord(c);

  const kind = clipKind(c.name);
  const icon = document.createElement("div");
  icon.className = "clip-kind " + kind;
  icon.title = CLIP_KIND_LABELS[kind];
  // Static per-kind markup, no clip data — innerHTML is safe here.
  icon.innerHTML = CLIP_KIND_ICONS[kind];

  // Leading cluster: the game icon (when known) sits beside the kind marker.
  const lead = document.createElement("div");
  lead.className = "clip-lead";
  const game = clipGameIcon(c);
  if (game) {
    const gi = document.createElement("img");
    gi.className = "clip-game-icon";
    gi.src = game.url;
    gi.alt = "";
    gi.title = game.label;
    // Fall back to a neutral glyph if the icon can't load (e.g. a plugin icon
    // asset that isn't present yet), so the badge stays visible.
    gi.addEventListener("error", () => {
      const ph = document.createElement("div");
      ph.className = "clip-game-icon placeholder";
      ph.title = game.label;
      ph.innerHTML = GENERIC_GAME_ICON; // static markup, safe
      gi.replaceWith(ph);
    });
    lead.appendChild(gi);
  }
  lead.appendChild(icon);

  const meta = document.createElement("div");
  const name = document.createElement("div");
  name.className = "name";
  const when = new Date(c.modified_unix * 1000);
  name.textContent = formatClipTitle(
    when.getMonth(), when.getDate(), when.getHours(), when.getMinutes());
  const info = document.createElement("div");
  info.className = "info";
  const markers = c.markers ? c.markers.markers : [];
  const digest = playerSummaryLabel(c.markers ? c.markers.player_summary : null) ||
    markerDigest(markers);
  info.textContent =
    `${fmtDur(c.duration_s)} · ${c.size_mb.toFixed(1)} MB · ` +
    fmtAgo(Date.now() / 1000, c.modified_unix) +
    (digest ? ` · ${digest}` : "") +
    (cloudRecord ? ` · cloud: ${cloudStatusLabel(cloudRecord)}` : "");
  meta.append(name, info);

  const cloud = document.createElement("button");
  cloud.className = "cloud";
  cloud.title = cloudRecord && cloudRecord.remote_url
    ? "Copy cloud link"
    : "Upload to Clipline Cloud";
  const busy = cloudRecord && ["queued", "uploading", "processing", "retrying"].includes(cloudRecord.upload_status);
  const uploaded = cloudRecord && cloudRecord.remote_url && cloudRecord.upload_status.startsWith("uploaded_");
  cloud.classList.toggle("uploaded", !!uploaded);
  cloud.classList.toggle("busy", !!busy);
  cloud.disabled = busy || (!uploaded && !cloudConnected());
  cloud.innerHTML = uploaded
    ? '<svg viewBox="0 0 24 24"><path d="M10.6 13.4a1 1 0 0 1 0-1.4l3.5-3.5a3 3 0 1 1 4.2 4.2l-1.5 1.5-1.4-1.4 1.5-1.5a1 1 0 1 0-1.4-1.4L12 13.4a1 1 0 0 1-1.4 0zm2.8-2.8a1 1 0 0 1 0 1.4l-3.5 3.5a3 3 0 1 1-4.2-4.2l1.5-1.5 1.4 1.4-1.5 1.5a1 1 0 1 0 1.4 1.4L12 10.6a1 1 0 0 1 1.4 0z"/></svg>'
    : '<svg viewBox="0 0 24 24"><path d="M12 3 6.5 8.5 8 10l3-3v10h2V7l3 3 1.5-1.5L12 3zM5 19h14v2H5v-2z"/></svg>';

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
  cloud.addEventListener("click", (ev) => {
    ev.stopPropagation();
    if (uploaded) copyCloudUrl(cloudRecord);
    else uploadClipToCloud(c);
  });

  el.append(lead, meta, cloud, del);
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
  resetCapturePreviewFrame();
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
  resetZoom();
  setTrim(0, clip.duration_s ?? (clip.markers ? clip.markers.duration_s : 0));
  renderOverviewMarkers();
  applyView({ start: 0, span: 0 });
  renderClips();
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
  resetZoom();
  updateViews();
  $("deck-status").textContent = "";
  $("stage-note").textContent = "";
  $("marker-layer").replaceChildren();
  renderClips();
}

/* ---- main pane views: empty / player / settings ---- */

let settingsOpen = false;

function updateViews() {
  $("settings-page").hidden = !settingsOpen;
  $("review-viewer").hidden = settingsOpen || !currentClip;
  $("review-empty").hidden = settingsOpen || !!currentClip;
  updateCapturePreview();
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

// The slice of the clip the timeline currently shows. Normalized every read so
// a stale zoom from a previous clip (or a shrunk duration) can never escape the
// bounds — when not zoomed this is just [0, duration].
function timelineView() {
  return clampView(zoomStart, zoomSpan, clipDuration());
}

function resetZoom() {
  zoomStart = 0;
  zoomSpan = 0;
}

// Central view setter: normalize the window, store it (collapsing a full-width
// span back to the zoomed-out sentinel), then re-render everything the window
// affects. Every zoom/pan/fit/follow path routes through here so the ruler,
// markers, track, and navigator can never drift out of sync.
function applyView(next) {
  const dur = clipDuration();
  const v = clampView(next.start, next.span, dur);
  zoomStart = v.start;
  zoomSpan = dur > 0 && v.span >= dur ? 0 : v.span;
  renderRuler();
  renderMarkers();
  paintTimeline();
}

// After a manual view change (wheel zoom/pan, zoom buttons, navigator drag) hold
// auto-follow off briefly, so playback doesn't immediately yank the view back to
// the playhead while the user is deliberately looking elsewhere.
const FOLLOW_SUPPRESS_MS = 1500;
let suppressFollowUntil = 0;
function noteViewActivity() {
  suppressFollowUntil = performance.now() + FOLLOW_SUPPRESS_MS;
}

// Keep the playhead in view while it moves on its own (playback, keyboard jumps,
// marker clicks). Gated on no active drag and a quiet period after a manual view
// change so it never pages out from under the user; only re-renders on a change.
function maybeFollow(playhead) {
  if (dragging || overviewDrag) return;
  if (performance.now() < suppressFollowUntil) return;
  if (!(zoomSpan > 0)) return; // zoomed out: the whole clip is already in view
  const v = timelineView();
  const next = followView(v.start, v.span, clipDuration(), playhead, DEFAULT_FOLLOW_MODE);
  if (Math.abs(next.start - v.start) > 1e-3 || Math.abs(next.span - v.span) > 1e-3) {
    applyView(next);
  }
}

/* ---- zoom / snap controls ---- */

// Zoom by a factor (<1 in, >1 out) anchored on the playhead so it stays in view.
function zoomAtPlayhead(factor) {
  const dur = clipDuration();
  if (!(dur > 0)) return;
  noteViewActivity();
  const v = timelineView();
  const ph = clampTime(video.currentTime || 0, dur);
  const frac = v.span > 0 ? Math.max(0, Math.min(1, (ph - v.start) / v.span)) : 0.5;
  applyView(zoomView(v.start, v.span, dur, frac, factor, MIN_VIEW_SPAN_S));
}

function zoomFit() {
  applyView({ start: 0, span: 0 });
}

// Frame the current trim selection (zoom to selection).
function zoomToSelection() {
  const dur = clipDuration();
  if (!(dur > 0)) return;
  noteViewActivity();
  applyView(viewForRange(trimStart, trimEnd, dur));
}

function setSnap(on) {
  snapEnabled = on;
  $("snap-toggle").classList.toggle("active", snapEnabled);
}

function toggleSnap() {
  setSnap(!snapEnabled);
}

// Best-effort clip frame rate: the recorder's configured fps, else a fine
// fallback. HTML <video> doesn't expose true fps, so frameStep degrades safely.
function clipFps() {
  return currentSettings && Number.isFinite(currentSettings.fps) ? currentSettings.fps : 0;
}

// Arrow keys jump several frames at once — one frame is too fine to navigate
// with, but the step stays frame-aligned (nice for landing trims on a frame).
const ARROW_STEP_FRAMES = 10;

function stepFrame(dir) {
  seekBy(dir * ARROW_STEP_FRAMES * frameStep(clipFps(), DEFAULT_FINE_STEP_S));
}

// Jump to the previous/next edit point (clip ends, trim edges, markers).
function jumpEdit(direction) {
  const points = editPoints(clipMarkers(), trimStart, trimEnd, clipDuration());
  const current = video.currentTime || 0;
  const target = direction > 0 ? nextMarker(points, current) : prevMarker(points, current);
  if (target) seekTo(target.t_s);
}

function paintTimeline() {
  const dur = clipDuration();
  const view = timelineView();
  const current = dur ? clampTime(video.currentTime || 0, dur) : 0;
  // Off-window positions fall outside 0–100% and are clipped by the track; the
  // dimmed trim ends are clamped so they fill the visible side they cover.
  const pct = (t) => percentForView(t, view.start, view.span);
  const edge = (t) => Math.max(0, Math.min(100, pct(t)));
  $("time-readout").textContent = `${fmtTenths(current)} / ${fmtTenths(dur)}`;
  $("playhead").style.left = `${pct(current)}%`;
  $("dim-in").style.width = `${edge(trimStart)}%`;
  $("dim-out").style.width = `${100 - edge(trimEnd)}%`;
  $("handle-in").style.left = `${pct(trimStart)}%`;
  $("handle-out").style.left = `${pct(trimEnd)}%`;
  // The slide strip only appears when there's an actual selection to move (not
  // the whole clip), so the top of the track still scrubs by default.
  const band = $("trim-band");
  const full = !dur || (trimStart <= 0.05 && trimEnd >= dur - 0.05);
  band.style.display = full ? "none" : "block";
  if (!full) {
    band.style.left = `${pct(trimStart)}%`;
    band.style.width = `${Math.max(0, pct(trimEnd) - pct(trimStart))}%`;
  }
  paintOverview();
}

// Cheap per-frame navigator update, in whole-clip coordinates: the trim band,
// the playhead, and the visible-window rectangle. The marker ticks are rebuilt
// separately (renderOverviewMarkers) only when the clip changes.
function paintOverview() {
  const win = $("overview-window");
  if (!win) return;
  const dur = clipDuration();
  const view = timelineView();
  const current = dur ? clampTime(video.currentTime || 0, dur) : 0;
  const a = percentFor(trimStart, dur);
  const b = percentFor(trimEnd, dur);
  $("overview-trim").style.left = `${a}%`;
  $("overview-trim").style.width = `${Math.max(0, b - a)}%`;
  $("overview-playhead").style.left = `${percentFor(current, dur)}%`;
  win.style.left = `${percentFor(view.start, dur)}%`;
  win.style.width = `${dur ? Math.max(0, Math.min(100, (view.span / dur) * 100)) : 100}%`;
}

// Rebuild the whole-clip marker ticks in the navigator. View-independent, so it
// runs on clip/marker change only — never per frame and never on zoom.
function renderOverviewMarkers() {
  const layer = $("overview-markers");
  if (!layer) return;
  layer.replaceChildren();
  const dur = clipDuration();
  for (const m of clipMarkers()) {
    const tick = document.createElement("i");
    tick.className = `ov-marker marker-${markerStyle(m.kind).cls}`;
    tick.style.left = `${percentFor(m.t_s, dur)}%`;
    layer.appendChild(tick);
  }
}

// timeupdate fires ~4 Hz; animate the playhead per-frame while playing.
// The same loop re-evaluates overlay fade (no timers to manage).
function animatePlayhead() {
  maybeFollow(video.currentTime || 0);
  paintTimeline();
  updateOverlay();
  if (!video.paused && !video.ended) rafId = requestAnimationFrame(animatePlayhead);
}

// Per-event glyphs for the marker pins, keyed by EventKind. Kept here (DOM
// layer) rather than in player-core.js so its tested {glyph,cls} contract stays
// untouched. Each draws in currentColor so the category tint (--mc) colors it.
const MARKER_ICONS = {
  ChampionKill: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M4.5 4.5 19.5 19.5M19.5 4.5 4.5 19.5"/><path d="M13 16 16 13M8 13 11 16"/><circle cx="19.5" cy="19.5" r="1.15" fill="currentColor" stroke="none"/><circle cx="4.5" cy="19.5" r="1.15" fill="currentColor" stroke="none"/></svg>`,
  FirstBlood: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"><path d="M12 3.5C12 3.5 18.5 11 18.5 15.5A6.5 6.5 0 1 1 5.5 15.5C5.5 11 12 3.5 12 3.5Z"/></svg>`,
  Multikill: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><path d="M6 11.5A6 6 0 0 1 18 11.5L18 14.5A1.4 1.4 0 0 1 16.6 15.9L16 15.9 16 18.5 8 18.5 8 15.9 7.4 15.9A1.4 1.4 0 0 1 6 14.5Z"/><circle cx="9.6" cy="12.2" r="1.5" fill="currentColor" stroke="none"/><circle cx="14.4" cy="12.2" r="1.5" fill="currentColor" stroke="none"/></svg>`,
  Ace: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"><path d="M12 3 14.12 9.51 20.97 9.51 15.42 13.54 17.55 20.05 12 16.02 6.45 20.05 8.58 13.54 3.03 9.51 9.88 9.51Z"/></svg>`,
  DragonKill: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><path d="M13 3C13.5 7 17 9 17 13.5A5 5 0 0 1 7 13.7C7 11.5 8.3 10.3 8.3 10.3C8.6 12 9.8 12.6 9.8 12.6C11 11.2 9.5 7.5 13 3Z"/></svg>`,
  HeraldKill: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"><path d="M3 12C6 6.5 18 6.5 21 12C18 17.5 6 17.5 3 12Z"/><circle cx="12" cy="12" r="2.7" fill="currentColor" stroke="none"/></svg>`,
  BaronKill: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"><path d="M4 18.5 4 8 8.5 11.5 12 5.5 15.5 11.5 20 8 20 18.5Z"/></svg>`,
  TurretKilled: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"><path d="M6 20.5 6 7 8.5 7 8.5 9 11 9 11 7 13 7 13 9 15.5 9 15.5 7 18 7 18 20.5Z"/></svg>`,
  InhibKilled: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><path d="M12 3 17 9 14 20.5 10 20.5 7 9Z"/><path d="M7 9 17 9M12 3 12 20.5"/></svg>`,
  FirstBrick: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><path d="M5 20.5 5 8.5 7 8.5 7 10 9 10 9 8.5 11 8.5 11 10 13 10 13 8.5 14.5 8.5 14.5 20.5Z"/><path d="M19 3.2 19.7 5.6 22.1 6.3 19.7 7 19 9.4 18.3 7 15.9 6.3 18.3 5.6Z" fill="currentColor" stroke="none"/></svg>`,
  GameStart: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M6.5 21 6.5 3"/><path d="M6.5 4 17 7 6.5 10"/></svg>`,
  MinionsSpawning: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="8.5"/><path d="M12 7.5 12 12 15 14"/></svg>`,
  GameEnd: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M7 4 17 4 17 7A5 5 0 0 1 7 7Z"/><path d="M7 5 4.5 5A2 2 0 0 0 7 8.7M17 5 19.5 5A2 2 0 0 1 17 8.7"/><path d="M12 12 12 16M8.5 19.5 15.5 19.5 15 16.5 9 16.5Z"/></svg>`,
  Other: `<svg viewBox="0 0 24 24" fill="currentColor" stroke="none"><circle cx="12" cy="12" r="3"/></svg>`,
};
// Unknown / future kinds fall back to a representative glyph for their category.
const MARKER_ICON_FALLBACK = {
  kill: MARKER_ICONS.ChampionKill,
  spree: MARKER_ICONS.Ace,
  objective: MARKER_ICONS.BaronKill,
  structure: MARKER_ICONS.TurretKilled,
  info: MARKER_ICONS.Other,
};
// Clicking a marker starts playback this many seconds before the event, so its
// lead-up plays rather than dropping the viewer right on the moment.
const MARKER_LEAD_S = 3;
// Game-authentic art for the kinds that actually reach the review timeline
// (is_timeline_marker). Used as a CSS mask so each silhouette still tints with
// its category color (--mc); kinds without art fall back to the SVGs above.
const MARKER_IMAGES = {
  ChampionKill: "assets/markers/kill.png",
  DragonKill: "assets/markers/dragon.png",
  BaronKill: "assets/markers/baron.png",
  TurretKilled: "assets/markers/turret.png",
};

function renderMarkers() {
  const layer = $("marker-layer");
  layer.replaceChildren();
  const view = timelineView();
  const markers = clipMarkers();
  for (const m of markers) {
    const left = percentForView(m.t_s, view.start, view.span);
    // The marker band isn't clipped like the track, so drop glyphs that would
    // ride outside the visible window (a small margin keeps edge glyphs whole).
    if (left < -2 || left > 102) continue;
    const style = markerStyle(m.kind);
    const marker = document.createElement("button");
    marker.className = `marker marker-${style.cls}`;
    marker.style.left = `${left}%`;
    marker.title = `${m.kind}${m.subtype ? ` (${m.subtype})` : ""} — ${m.actor}${m.victim ? " → " + m.victim : ""} @ ${m.t_s.toFixed(1)}s`;

    const glyph = document.createElement("span");
    glyph.className = "glyph";
    const img = MARKER_IMAGES[m.kind];
    if (img) {
      glyph.classList.add("img");
      glyph.style.setProperty("--marker-img", `url("${img}")`);
    } else {
      glyph.innerHTML = MARKER_ICONS[m.kind] || MARKER_ICON_FALLBACK[style.cls] || MARKER_ICONS.Other;
    }
    const hair = document.createElement("span");
    hair.className = "hair";
    marker.append(glyph, hair);

    marker.addEventListener("pointerdown", (ev) => ev.stopPropagation());
    marker.addEventListener("click", (ev) => {
      ev.stopPropagation();
      // Start a beat before the event so its lead-up plays, then roll.
      seekTo(m.t_s - MARKER_LEAD_S);
      video.play().catch(() => syncPlayState());
    });
    layer.appendChild(marker);
  }
  $("marker-count").textContent = markerSummary(markers);
  $("prev-marker").disabled = !markers.length;
  $("next-marker").disabled = !markers.length;
}

function renderRuler() {
  const root = $("ruler");
  root.replaceChildren();
  const view = timelineView();
  if (!(view.span > 0)) return;
  const viewEnd = view.start + view.span;
  const pct = (t) => percentForView(t, view.start, view.span);
  const marks = rulerMarksRange(view.start, view.span, 8);
  // Minor ticks between the labeled majors give the ruler a fine, precise feel.
  if (marks.length >= 2) {
    const step = marks[1].t - marks[0].t;
    const minorStep = step / 3;
    const isMajor = (t) => marks.some((m) => Math.abs(m.t - t) < minorStep / 2);
    const firstMinor = Math.ceil(view.start / minorStep - 1e-9) * minorStep;
    for (let t = firstMinor; t <= viewEnd + 1e-6; t += minorStep) {
      if (t <= 0 || isMajor(t)) continue;
      const tick = document.createElement("i");
      tick.className = "tick";
      tick.style.left = `${pct(t)}%`;
      root.appendChild(tick);
    }
  }
  marks.forEach((mark) => {
    const tick = document.createElement("i");
    tick.className = "tick major";
    tick.style.left = `${pct(mark.t)}%`;
    root.appendChild(tick);
    const lab = document.createElement("span");
    // The 0:00 label hugs the left edge (no centering) only when it sits there.
    const atLeftEdge = view.start === 0 && mark.t <= 1e-6;
    lab.className = atLeftEdge ? "lab first" : "lab";
    lab.style.left = `${pct(mark.t)}%`;
    lab.textContent = mark.label;
    root.appendChild(lab);
  });
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
  maybeFollow(t);
  paintTimeline();
}

video.addEventListener("seeked", () => {
  if (pendingSeek != null) {
    const t = pendingSeek;
    pendingSeek = null;
    video.currentTime = t;
  }
  maybeFollow(video.currentTime || 0);
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
// Snap targets snapshotted at pointerdown so a drag never snaps to its own
// moving position (the dragged edge and the playhead are excluded up front).
let dragCandidates = [];
// Sliding the whole selection: offset from pointer to selection start, the click
// time, and whether the pointer moved enough to count as a drag (vs a seek).
let slideGrab = 0;
let slideClickT = 0;
let slideStartX = 0;
let slideMoved = false;
const SLIDE_THRESHOLD_PX = 4;

function clearSnapFeedback() {
  $("playhead").classList.remove("snapped");
  $("handle-in").classList.remove("snapped");
  $("handle-out").classList.remove("snapped");
  $("trim-band").classList.remove("snapped");
}

function startDrag(kind, ev) {
  if (!currentClip) return;
  dragging = kind;
  // Scrub paused so every pointer position shows its frame, then restore.
  resumeAfterDrag = !video.paused;
  if (resumeAfterDrag) video.pause();
  // Exclude the element(s) being moved so a drag never snaps to itself.
  const playhead = video.currentTime || 0;
  let exclude;
  if (kind === "scrub") {
    exclude = ["playhead"]; // the playhead rides the pointer
  } else if (kind === "slide") {
    exclude = ["in", "out"]; // both edges move together
  } else {
    // Trim edge: always drop the dragged edge. The playhead rides that edge once
    // the drag starts, so drop it too ONLY if it's already within snap range of
    // the edge (else the handle would stick to its own start) — a playhead parked
    // elsewhere stays a useful snap target.
    exclude = [kind];
    const rect = $("timeline").getBoundingClientRect();
    const v = timelineView();
    const pps = rect.width && v.span > 0 ? rect.width / v.span : 0;
    const tol = pps > 0 ? SNAP_THRESHOLD_PX / pps : 0.05;
    const edge = kind === "in" ? trimStart : trimEnd;
    if (Math.abs(playhead - edge) <= tol) exclude.push("playhead");
  }
  dragCandidates = snapCandidates(clipDuration(), clipMarkers(), playhead, trimStart, trimEnd, exclude);
  if (kind === "slide") {
    const rect = $("timeline").getBoundingClientRect();
    const v = timelineView();
    const t = timelineTimeView(ev.clientX, rect.left, rect.width, v.start, v.span, clipDuration());
    slideGrab = t - trimStart;
    slideClickT = t;
    slideStartX = ev.clientX;
    slideMoved = false;
    $("trim-band").classList.add("grabbing");
  }
  $("timeline").setPointerCapture(ev.pointerId);
  moveDrag(ev);
}

function moveDrag(ev) {
  if (!dragging) return;
  const rect = $("timeline").getBoundingClientRect();
  const view = timelineView();
  const dur = clipDuration();
  const rawT = timelineTimeView(ev.clientX, rect.left, rect.width, view.start, view.span, dur);
  const pps = rect.width && view.span > 0 ? rect.width / view.span : 0;
  const doSnap = snapEnabled && !ev.altKey && pps > 0;
  clearSnapFeedback();

  if (dragging === "slide") {
    // Hold still and release to seek; move past the threshold to start sliding.
    if (!slideMoved && Math.abs(ev.clientX - slideStartX) <= SLIDE_THRESHOLD_PX) return;
    slideMoved = true;
    // Move the whole selection, keeping its length. Snap whichever edge lands
    // closest to a salient time so either end can lock cleanly.
    const len = trimEnd - trimStart;
    let newStart = rawT - slideGrab;
    let snapped = false;
    if (doSnap) {
      const a = snapTime(newStart, dragCandidates, pps, SNAP_THRESHOLD_PX);
      const b = snapTime(newStart + len, dragCandidates, pps, SNAP_THRESHOLD_PX);
      const da = a.snapped ? Math.abs(a.t - newStart) : Infinity;
      const db = b.snapped ? Math.abs(b.t - (newStart + len)) : Infinity;
      if (da <= db && a.snapped) { newStart = a.t; snapped = true; }
      else if (b.snapped) { newStart = b.t - len; snapped = true; }
    }
    const next = slideTrim(trimStart, trimEnd, newStart, dur);
    setTrim(next.start, next.end);
    if (snapped) $("trim-band").classList.add("snapped");
    return;
  }

  let t = rawT;
  let snapped = false;
  if (doSnap) {
    const res = snapTime(t, dragCandidates, pps, SNAP_THRESHOLD_PX);
    t = res.t;
    snapped = res.snapped;
  }
  if (dragging === "scrub") {
    if (snapped) $("playhead").classList.add("snapped");
    seekTo(t);
  } else {
    if (snapped) $(dragging === "in" ? "handle-in" : "handle-out").classList.add("snapped");
    const next = trimDrag(dragging, t, trimStart, trimEnd, dur);
    setTrim(next.start, next.end);
    // The playhead rides the dragged edge — you trim on the frame you see.
    seekTo(dragging === "in" ? next.start : next.end);
  }
}

function endDrag() {
  if (!dragging) return;
  // A press-and-release on the selection without dragging just seeks there.
  const clickSeek = dragging === "slide" && !slideMoved;
  dragging = null;
  dragCandidates = [];
  clearSnapFeedback();
  $("trim-band").classList.remove("grabbing");
  if (clickSeek) seekTo(slideClickT);
  if (resumeAfterDrag) {
    resumeAfterDrag = false;
    video.play().catch(() => syncPlayState());
  }
}

// Higher = faster zoom per wheel notch. e^(±notch·sensitivity) is the span
// multiplier, so it zooms by the same ratio whichever way you scroll.
const ZOOM_SENSITIVITY = 0.0015;

// Scroll over the timeline to zoom, keeping the clip moment under the cursor
// pinned. Scroll up (deltaY < 0) zooms in, down zooms back out.
function onTimelineWheel(ev) {
  const dur = clipDuration();
  if (!currentClip || !(dur > 0)) return;
  ev.preventDefault();
  noteViewActivity();
  const rect = $("timeline").getBoundingClientRect();
  if (!rect.width) return;
  // Normalize line/page wheels (Firefox-style) to roughly pixel scale.
  const unit = ev.deltaMode === 1 ? 33 : ev.deltaMode === 2 ? rect.width : 1;
  const view = timelineView();
  // Shift+wheel, or a genuinely horizontal trackpad gesture, pans instead of
  // zooming. Requiring |deltaX| > |deltaY| keeps trackpad noise during a
  // vertical scroll from misfiring a pan.
  if (ev.shiftKey || Math.abs(ev.deltaX) > Math.abs(ev.deltaY)) {
    const raw = ev.shiftKey ? ev.deltaY || ev.deltaX : ev.deltaX;
    const seconds = ((raw * unit) / rect.width) * view.span;
    applyView(panView(view.start, view.span, dur, seconds));
    return;
  }
  const anchorFrac = (ev.clientX - rect.left) / rect.width;
  const factor = Math.max(0.5, Math.min(2, Math.exp(ev.deltaY * unit * ZOOM_SENSITIVITY)));
  applyView(zoomView(view.start, view.span, dur, anchorFrac, factor, MIN_VIEW_SPAN_S));
}

/* ---- navigator (whole-clip minimap) drag: body pans, grips zoom ---- */

// Clip time under the pointer in the whole-clip navigator.
function overviewTime(ev) {
  const rect = $("overview").getBoundingClientRect();
  const dur = clipDuration();
  if (!rect.width || !dur) return 0;
  const x = Math.max(0, Math.min(rect.width, ev.clientX - rect.left));
  return (x / rect.width) * dur;
}

function onOverviewPointerDown(ev) {
  if (!currentClip || !(clipDuration() > 0)) return;
  ev.preventDefault();
  const dur = clipDuration();
  const v = timelineView();
  const t = overviewTime(ev);
  if (ev.target === $("overview-window-l")) {
    overviewDrag = { mode: "left", pointerId: ev.pointerId };
    moveOverviewDrag(ev);
  } else if (ev.target === $("overview-window-r")) {
    overviewDrag = { mode: "right", pointerId: ev.pointerId };
    moveOverviewDrag(ev);
  } else if (ev.target === $("overview-window")) {
    // Grab the box where you clicked it and pan, keeping that point under the cursor.
    overviewDrag = { mode: "pan", grab: t - v.start, pointerId: ev.pointerId };
  } else {
    // Clicking the empty track jumps the window to center on the click, then pans.
    const nv = clampView(t - v.span / 2, v.span, dur);
    applyView(nv);
    overviewDrag = { mode: "pan", grab: t - nv.start, pointerId: ev.pointerId };
  }
  $("overview").setPointerCapture(ev.pointerId);
  $("overview-window").classList.add("grabbing");
}

function moveOverviewDrag(ev) {
  if (!overviewDrag) return;
  const dur = clipDuration();
  const v = timelineView();
  const t = overviewTime(ev);
  if (overviewDrag.mode === "pan") {
    applyView(clampView(t - overviewDrag.grab, v.span, dur));
  } else {
    applyView(setViewEdge(v.start, v.span, dur, overviewDrag.mode, t));
  }
}

function endOverviewDrag() {
  if (!overviewDrag) return;
  overviewDrag = null;
  $("overview-window").classList.remove("grabbing");
  noteViewActivity(); // don't snap back to the playhead the instant the drag ends
}

// Navigator scroll pans the visible window left/right. The strip spans the whole
// clip, so map pixels scrolled to clip seconds (no-op when fully zoomed out).
function onOverviewWheel(ev) {
  const dur = clipDuration();
  if (!currentClip || !(dur > 0)) return;
  ev.preventDefault();
  noteViewActivity();
  const rect = $("overview").getBoundingClientRect();
  if (!rect.width) return;
  const unit = ev.deltaMode === 1 ? 33 : ev.deltaMode === 2 ? rect.width : 1;
  const raw = Math.abs(ev.deltaX) > Math.abs(ev.deltaY) ? ev.deltaX : ev.deltaY;
  const view = timelineView();
  applyView(panView(view.start, view.span, dur, ((raw * unit) / rect.width) * dur));
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

function confirmQuit() {
  return new Promise((resolve) => {
    const dlg = $("quit-dialog");
    const finish = (ok) => {
      dlg.removeEventListener("close", onClose);
      if (dlg.open) dlg.close();
      resolve(ok);
    };
    const onClose = () => finish(false); // Esc / backdrop paths
    dlg.addEventListener("close", onClose);
    $("quit-cancel").onclick = () => finish(false);
    $("quit-accept").onclick = () => finish(true);
    dlg.showModal();
  });
}

async function requestWindowClose() {
  if (currentSettings && currentSettings.close_to_tray === false) {
    if (!(await confirmQuit())) return;
  }
  await appWindow.close();
}

function setUpdateStatus(message) {
  $("update-status").textContent = message;
}

function updateUpToDateStatus(update) {
  const version = update.current_version ? ` ${update.current_version}` : "";
  return `${update.channel_label}${version} is up to date`;
}

function updateNotesPreview(notes) {
  const text = String(notes || "").trim();
  if (!text) return "";
  return text.length > 220 ? `${text.slice(0, 217)}...` : text;
}

function showUpdateDialog(update) {
  pendingUpdate = update;
  $("update-install").disabled = false;
  $("update-cancel").disabled = false;
  $("update-dialog-title").textContent = `${update.channel_label} update available`;
  $("update-dialog-body").textContent =
    `Clipline ${update.version} is available. Current version: ${update.current_version}.`;
  $("update-dialog-notes").textContent = updateNotesPreview(update.notes);
  $("update-dialog").showModal();
}

async function checkForUpdates({ manual = false } = {}) {
  if (updateCheckRunning) return;
  updateCheckRunning = true;
  if (manual) setUpdateStatus("checking...");
  try {
    const update = await invoke("check_for_updates");
    if (update.available) {
      setUpdateStatus(`${update.channel_label} ${update.version} available`);
      showUpdateDialog(update);
    } else if (manual) {
      setUpdateStatus(update.status || updateUpToDateStatus(update));
    }
  } catch (e) {
    if (manual) {
      setUpdateStatus(String(e));
    } else {
      console.warn("update check failed:", e);
    }
  } finally {
    updateCheckRunning = false;
  }
}

async function installPendingUpdate() {
  $("update-install").disabled = true;
  $("update-cancel").disabled = true;
  setUpdateStatus("installing update...");
  try {
    await invoke("install_update");
  } catch (e) {
    $("update-install").disabled = false;
    $("update-cancel").disabled = false;
    setUpdateStatus(String(e));
  }
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

async function copyClipToClipboard() {
  if (!currentClip) return;
  $("copy-clip").disabled = true;
  $("error").textContent = "";
  $("deck-status").textContent = "";
  try {
    await invoke("copy_clip_to_clipboard", { path: currentClip.path });
    $("deck-status").textContent = "clip copied to clipboard";
  } catch (e) {
    $("deck-status").textContent = "";
    $("error").textContent = e;
  } finally {
    $("copy-clip").disabled = false;
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

async function reloadSettings() {
  const settings = await invoke("get_settings");
  fillSettings(settings);
  if (clipsCache.length) renderClips();
}

async function connectCloud() {
  $("cloud-connect-status").textContent = "connecting...";
  $("error").textContent = "";
  try {
    await invoke("cloud_connect", {
      request: {
        host_url: $("cloud-host-url").value.trim(),
        username: $("cloud-username").value.trim(),
        password: $("cloud-password").value,
        device_name: "Clipline Desktop",
        plain_http_confirmed: $("cloud-http-confirm").checked,
        default_visibility: $("cloud-default-visibility").value,
      },
    });
    $("cloud-connect-status").textContent = "connected";
    await reloadSettings();
  } catch (e) {
    $("cloud-connect-status").textContent = String(e);
  }
}

async function disconnectCloud() {
  $("cloud-connect-status").textContent = "";
  $("error").textContent = "";
  try {
    await invoke("cloud_disconnect");
    await reloadSettings();
  } catch (e) {
    $("cloud-connect-status").textContent = String(e);
  }
}

async function copyCloudUrl(record) {
  if (!record || !record.remote_url) return;
  $("deck-status").textContent = "";
  $("error").textContent = "";
  try {
    await navigator.clipboard.writeText(record.remote_url);
    $("deck-status").textContent = "cloud link copied";
  } catch (e) {
    $("error").textContent = String(e);
  }
}

async function uploadClipToCloud(clip) {
  if (!clip || !cloudConnected()) return;
  $("deck-status").textContent = "uploading to cloud...";
  $("error").textContent = "";
  try {
    const result = await invoke("upload_clip_to_cloud", {
      path: clip.path,
      visibility: cloudSettings().default_visibility || "private",
    });
    if (result && result.record) {
      upsertCloudUploadRecord(result.record);
      if (result.record.remote_url && result.record.upload_status.startsWith("uploaded_")) {
        $("deck-status").textContent = "cloud upload ready";
      } else if (result.record.upload_status === "failed") {
        $("deck-status").textContent = "";
        $("error").textContent = result.record.error || "cloud upload failed";
      } else {
        $("deck-status").textContent = "cloud upload processing";
      }
    }
    await refresh();
  } catch (e) {
    $("deck-status").textContent = "";
    $("error").textContent = String(e);
    renderClips();
  }
}

/* ---- backend events ---- */

listen("status", (e) => {
  const s = e.payload;
  const wasRecording = recordingActive;
  recordingActive = s.recording;
  fullSessionRecordingActive = Boolean(s.full_session);
  if (wasRecording !== recordingActive) resetCapturePreviewFrame();
  $("dot").className = "dot" + (s.recording ? " on" : "");
  $("rail-dot").className = "dot" + (s.recording ? " on" : "");
  updateCaptureStatus();
});

listen("saved", (e) => {
  $("error").textContent = "";
  const s = e.payload;
  const savedKind = s.full_session ? "session" : "replay";
  $("notice").textContent = s.gc_deleted
    ? `cleaned up ${s.gc_deleted} old clip${s.gc_deleted > 1 ? "s" : ""} (${fmtBytes(s.gc_freed_bytes)})`
    : `saved ${fmtDur(s.seconds)} ${savedKind}`;
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

listen("preview-frame", (e) => {
  if (!previewShouldRun()) return;
  const frame = e.payload || {};
  if (!frame.data_url) return;
  $("capture-preview-image").src = frame.data_url;
  $("capture-preview-image").hidden = false;
  previewHasFrame = true;
  updateCapturePreview();
});

listen("game-detection", (e) => {
  activeDetectedGame = e.payload || null;
  resetCapturePreviewFrame();
  updateCaptureStatus();
  updateGameDetectionStatus();
  updateCapturePreview();
});

listen("cloud-upload-progress", (e) => {
  const progress = e.payload || {};
  upsertCloudProgress(progress);
  if (progress.error) {
    $("error").textContent = progress.error;
  } else if (progress.upload_status === "uploading") {
    const total = Number(progress.file_size_bytes) || 0;
    const done = Number(progress.received_size_bytes) || 0;
    $("deck-status").textContent = total > 0
      ? `cloud upload ${Math.round((done / total) * 100)}%`
      : "cloud upload in progress";
  } else if (progress.upload_status === "processing") {
    $("deck-status").textContent = "cloud upload processing";
  }
  renderClips();
});

/* ---- wiring ---- */

$("save").addEventListener("click", () => invoke("save_replay"));
$("capture-status").addEventListener("click", toggleRecording);
$("rail-status").addEventListener("click", toggleRecording);
$("set-capture").addEventListener("change", () => {
  captureTargetDirty = true;
  syncCaptureFields();
});
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
$("check-updates").addEventListener("click", () => checkForUpdates({ manual: true }));
$("update-install").addEventListener("click", installPendingUpdate);
$("update-cancel").addEventListener("click", () => {
  pendingUpdate = null;
  $("update-dialog").close();
});
$("set-replay-disk-enabled").addEventListener("change", syncReplayStorageFields);
$("set-replay-disk-quota").addEventListener("input", syncReplayStorageFields);
$("set-replay-disk-quota").addEventListener("change", syncReplayStorageFields);
for (const id of ["cloud-default-visibility", "cloud-delete-local-after-upload"]) {
  $(id).addEventListener("change", () => {
    $("settings-status").textContent = "cloud settings changed - save to apply";
  });
}
$("cloud-connect").addEventListener("click", connectCloud);
$("cloud-disconnect").addEventListener("click", disconnectCloud);
$("set-games-auto-detect").addEventListener("change", updateGameDetectionStatus);
for (const id of ["set-buffer", "set-replay", "set-encoder", "set-output-resolution", "set-bitrate", "set-fps"]) {
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
document.querySelector(".titlebar").addEventListener("pointerdown", (ev) => {
  if (ev.button !== 0 || ev.target.closest(".titlebar-btn")) return;
  armPreviewWindowMovePause(ev);
});
window.addEventListener("pointermove", maybePausePreviewForWindowMove);
window.addEventListener("pointerup", resumePreviewAfterWindowMove);
window.addEventListener("pointercancel", resumePreviewAfterWindowMove);
window.addEventListener("focus", () => {
  windowFocused = true;
  previewWindowMovePaused = false;
  clearTimeout(previewWindowMoveTimer);
  previewRequested = false;
  updateCapturePreview();
});
window.addEventListener("blur", () => {
  windowFocused = false;
  clearTimeout(previewWindowMoveTimer);
  previewWindowMovePaused = false;
  resetCapturePreviewFrame();
  updateCapturePreview();
});
document.addEventListener("visibilitychange", () => {
  windowFocused = !document.hidden && document.hasFocus();
  if (windowFocused) {
    previewWindowMovePaused = false;
    clearTimeout(previewWindowMoveTimer);
    previewRequested = false;
  } else {
    resetCapturePreviewFrame();
  }
  updateCapturePreview();
});
$("settings-save").addEventListener("click", async () => {
  $("settings-status").textContent = "";
  $("error").textContent = "";
  try {
    const saved = await invoke("save_settings", { settings: readSettings() });
    resetCapturePreviewFrame();
    fillSettings(saved);
    updateCapturePreview();
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
    // Duration is now exact: rebuild the whole-clip navigator and re-render.
    renderOverviewMarkers();
    applyView({ start: zoomStart, span: zoomSpan });
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
$("copy-clip").addEventListener("click", copyClipToClipboard);

$("zoom-in").addEventListener("click", () => zoomAtPlayhead(0.5));
$("zoom-out").addEventListener("click", () => zoomAtPlayhead(2));
// Plain click frames the trim selection (the editing default); Shift-click fits
// the whole clip — mirroring \ and Shift+\.
$("zoom-fit").addEventListener("click", (ev) => (ev.shiftKey ? zoomFit() : zoomToSelection()));
$("snap-toggle").addEventListener("click", toggleSnap);

// Keyboard shortcuts guide — the corner "K" keycap opens it; click the X or the
// backdrop (or press Esc, which the modal dialog handles) to close.
$("keys-help").addEventListener("click", () => $("keys-dialog").showModal());
$("keys-close").addEventListener("click", () => $("keys-dialog").close());
$("keys-dialog").addEventListener("click", (ev) => {
  if (ev.target === $("keys-dialog")) $("keys-dialog").close();
});

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
  else if (ev.target === $("trim-band")) startDrag("slide", ev);
  else startDrag("scrub", ev);
});
$("timeline").addEventListener("pointermove", moveDrag);

// Scroll to zoom. Bound on the stack (covers the track and the marker band
// above it via bubbling) and the ruler below; passive:false so we can stop the
// page from scrolling instead.
document
  .querySelector(".timeline-stack")
  .addEventListener("wheel", onTimelineWheel, { passive: false });
$("ruler").addEventListener("wheel", onTimelineWheel, { passive: false });

// Navigator (whole-clip minimap): drag the box to pan, its grips to zoom.
$("overview").addEventListener("pointerdown", onOverviewPointerDown);
$("overview").addEventListener("pointermove", moveOverviewDrag);
$("overview").addEventListener("pointerup", endOverviewDrag);
$("overview").addEventListener("pointercancel", endOverviewDrag);
$("overview").addEventListener("lostpointercapture", endOverviewDrag);
$("overview").addEventListener("wheel", onOverviewWheel, { passive: false });

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
  if ($("confirm-dialog").open || $("quit-dialog").open || $("update-dialog").open || $("keys-dialog").open) return; // a dialog owns the keyboard
  if (ev.code === "Escape" && settingsOpen) {
    ev.preventDefault();
    toggleSettings(false);
    return;
  }
  if (settingsOpen) return; // player shortcuts are inert behind the page
  const tag = ev.target && ev.target.tagName;
  if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") return;
  // "?" opens the shortcuts guide from anywhere in the player (clip or not).
  if (ev.code === "Slash" && ev.shiftKey) {
    ev.preventDefault();
    $("keys-dialog").showModal();
    return;
  }
  if (!currentClip) return;
  const intent = keyIntent(ev.code, ev.shiftKey);
  if (!intent) return;
  ev.preventDefault();
  noteActivity();
  switch (intent.kind) {
    case "toggle-play": togglePlay(); break;
    case "seek-by": seekBy(intent.seconds); break;
    case "step-frame": stepFrame(intent.dir); break;
    case "seek-to": seekTo(intent.seconds); break;
    case "seek-to-end": seekTo(clipDuration()); break;
    case "set-in": setTrim(video.currentTime || 0, trimEnd); break;
    case "set-out": setTrim(trimStart, video.currentTime || 0); break;
    case "next-marker": jumpMarker(1); break;
    case "prev-marker": jumpMarker(-1); break;
    case "next-edit": jumpEdit(1); break;
    case "prev-edit": jumpEdit(-1); break;
    case "zoom": zoomAtPlayhead(intent.factor); break;
    case "zoom-fit": zoomFit(); break;
    case "zoom-selection": zoomToSelection(); break;
    case "toggle-snap": toggleSnap(); break;
    case "toggle-focus": toggleRail(); break;
    case "close": closeReview(); break;
  }
});

/* ---- boot ---- */

updateViews();
syncPlayState();
syncVolume();
syncAllRangeProgress();
async function loadInitialSettings() {
  await loadGamePlugins();
  let settings = await invoke("get_settings");
  // The registry Run key is the ground truth for startup. Reconcile the UI
  // in case the entry was changed externally since the last save.
  try {
    settings = { ...settings, open_on_startup: await invoke("get_autostart_status") };
  } catch (e) {
    console.warn("could not read autostart status:", e);
  }
  fillSettings(settings);
  window.setTimeout(() => checkForUpdates({ manual: false }), 1500);
  // Custom-game icons live in settings; refresh clip badges once they load.
  if (clipsCache.length) renderClips();
}
loadInitialSettings().catch((e) => $("error").textContent = e);
loadDisplays();
loadAudioDevices();
loadVideoEncoders();
refresh();
refreshMemoryUsage();
setInterval(refreshMemoryUsage, 2000);
