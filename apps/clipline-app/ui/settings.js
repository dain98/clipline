// Settings page: form I/O, capture region, devices, games.
function cloneSettings(settings) {
  return settings ? JSON.parse(JSON.stringify(settings)) : null;
}

function settingsFormSource() {
  return settingsDraft || currentSettings || {};
}

function syncSettingsDraftFromForm() {
  settingsDraft = readSettings();
  return settingsDraft;
}

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
  settingsDraft = cloneSettings(currentSettings);
  regionState = s.capture_region ?? regionState;
  captureTargetDirty = false;
  renderCaptureTargetSelect();
  $("set-games-auto-detect").checked = !!games.auto_detect;
  $("set-output-enabled").checked = !!audio.output_enabled;
  $("set-audio-split-output").checked = audio.split_output_by_process === true;
  $("set-output-volume").value = String(Number.isFinite(audio.output_volume) ? audio.output_volume : 1);
  $("set-mic-enabled").checked = !!audio.mic_enabled;
  $("set-mic-volume").value = String(Number.isFinite(audio.mic_volume) ? audio.mic_volume : 1);
  $("set-mic-mono").checked = (audio.mic_channels || "mono") === "mono";
  $("set-buffer").value = Number(s.buffer_seconds) || ((Number(s.replay_window_s) || 60) + 15);
  $("set-replay").value = Math.min(120, Number(s.replay_window_s) || 60);
  $("set-backend").value = s.capture_backend || "auto";
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
  updateHotkeyLabels(s.hotkey);
  $("set-open-on-startup").checked = !!s.open_on_startup;
  $("set-close-to-tray").checked = s.close_to_tray !== false;
  $("set-minimize-to-tray").checked = !!s.minimize_to_tray;
  $("set-legacy-timeline-editor").checked = !!s.legacy_timeline_editor;
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
  syncUploadClipButton();
  applyTimelineEditorPreference();
  renderClips();
}

function readSettings() {
  const replay = Number($("set-replay").value);
  const capture = selectedCaptureSettings();
  const source = settingsFormSource();
  const preserveLegacyWindow =
    !captureTargetDirty
    && source.capture_mode === "window_title"
    && String(source.window_title || "").trim().length > 0;
  return {
    capture_mode: preserveLegacyWindow ? "window_title" : capture.capture_mode,
    capture_backend: $("set-backend").value,
    window_title: preserveLegacyWindow ? source.window_title : "",
    capture_region: preserveLegacyWindow
      ? (source.capture_region || capture.capture_region)
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
      split_output_by_process: $("set-audio-split-output").checked,
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
    legacy_timeline_editor: $("set-legacy-timeline-editor").checked,
    update_channel: $("set-update-channel").value,
    cloud: readCloudSettings(),
  };
}

function defaultAudioSettings() {
  return {
    output_enabled: true,
    output_device_id: null,
    output_volume: 1,
    split_output_by_process: false,
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
    connected_display_name: null,
    credential_target: null,
    default_visibility: "private",
    delete_local_after_upload: false,
    auto_upload_rules: false,
    uploads: {},
  };
}

function defaultGamePluginSettings(plugin) {
  return {
    enabled: plugin ? plugin.default_enabled !== false : true,
    recording_mode: normalizeGameRecordingMode(
      plugin && plugin.default_recording_mode ? plugin.default_recording_mode : "full_session"
    ),
    review: defaultGamePluginReviewSettings(),
  };
}

function defaultGamePluginReviewSettings() {
  return PlayerCore.normalizeGameReviewSettings(null);
}

function normalizeGamePluginReviewSettings(settings) {
  return PlayerCore.normalizeGameReviewSettings(settings);
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
    review: normalizeGamePluginReviewSettings(settings && settings.review ? settings.review : defaults.review),
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

function gamePluginReviewInputs(plugin) {
  return Array.from(document.querySelectorAll(`[data-game-plugin-review-setting="${plugin.id}"]`));
}

function readGamePluginReviewSettings(plugin, fallback) {
  const review = normalizeGamePluginReviewSettings(fallback);
  const master = document.querySelector(`[data-game-plugin-review-enabled="${plugin.id}"]`);
  const next = normalizeGamePluginReviewSettings({
    ...review,
    enabled: master ? master.checked : review.enabled,
  });
  for (const input of gamePluginReviewInputs(plugin)) {
    const group = input.dataset.reviewGroup;
    const key = input.dataset.reviewKey;
    if (!next[group] || !Object.prototype.hasOwnProperty.call(next[group], key)) continue;
    next[group][key] = input.checked;
  }
  return next;
}

function readGamePluginSettings() {
  const source = settingsFormSource();
  const next = {
    ...normalizeGamePluginSettingsMap(
      source.games ? source.games.plugins : {}
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
      review: readGamePluginReviewSettings(plugin, existing.review),
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

function refreshReviewForSettingsChange() {
  if (clipsCache.length) renderClips();
  if (!currentClip) return;
  if (typeof renderOverviewMarkers === "function") renderOverviewMarkers();
  if (typeof renderMarkers === "function") renderMarkers();
  renderGameEventRail(currentClip);
  renderGameMetadataPanel(currentClip);
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

const GAME_REVIEW_GROUPS = [
  {
    id: "match_events",
    label: "Match events",
    options: [
      ["user_kills", "User kills"],
      ["user_deaths", "User deaths"],
      ["user_assists", "User assists"],
      ["team_kills", "Ally kills"],
      ["team_deaths", "Ally deaths"],
      ["enemy_kills", "Enemy kills"],
      ["enemy_deaths", "Enemy deaths"],
      ["objectives", "Objectives"],
      ["turrets", "Turrets"],
    ],
  },
  {
    id: "timeline_markers",
    label: "Timeline markers",
    options: [
      ["user_kills", "User kills"],
      ["user_deaths", "User deaths"],
      ["user_assists", "User assists"],
      ["objectives", "Objectives"],
      ["turrets", "Turrets"],
    ],
  },
];

function syncGamePluginReviewControls(plugin) {
  const settings = gamePluginSetting(plugin);
  const reviewEnabled = settings.review.enabled;
  const groups = document.querySelectorAll(`[data-game-plugin-review-group="${plugin.id}"]`);
  groups.forEach((group) => {
    const groupName = group.dataset.reviewGroup;
    const groupEnabled = Boolean(settings.review[groupName] && settings.review[groupName].enabled);
    group.classList.toggle("disabled", !reviewEnabled || !groupEnabled);
    group.querySelectorAll("input").forEach((input) => {
      if (input.dataset.reviewKey === "enabled") {
        input.disabled = !reviewEnabled;
      } else {
        input.disabled = !reviewEnabled || !groupEnabled;
      }
    });
  });
}

function updateGamePluginReviewSetting(plugin) {
  const existing = gamePluginSetting(plugin);
  gamePluginSettings[plugin.id] = normalizeGamePluginSettings({
    ...existing,
    review: readGamePluginReviewSettings(plugin, existing.review),
  }, plugin);
  syncGamePluginReviewControls(plugin);
  refreshReviewForSettingsChange();
}

function renderReviewCheckbox(plugin, groupId, key, labelText, checked) {
  const label = document.createElement("label");
  label.className = "check-line";
  const input = document.createElement("input");
  input.type = "checkbox";
  input.checked = checked;
  input.dataset.gamePluginReviewSetting = plugin.id;
  input.dataset.reviewGroup = groupId;
  input.dataset.reviewKey = key;
  input.addEventListener("change", () => updateGamePluginReviewSetting(plugin));
  const text = document.createElement("span");
  text.textContent = labelText;
  label.append(input, text);
  return label;
}

function renderGamePluginReviewControls(plugin, settings) {
  const review = normalizeGamePluginReviewSettings(settings.review);
  const root = document.createElement("div");
  root.className = "game-profile-review";

  const master = document.createElement("label");
  master.className = "check-line game-review-master";
  const masterInput = document.createElement("input");
  masterInput.type = "checkbox";
  masterInput.checked = review.enabled;
  masterInput.dataset.gamePluginReviewEnabled = plugin.id;
  masterInput.addEventListener("change", () => updateGamePluginReviewSetting(plugin));
  const masterText = document.createElement("span");
  masterText.textContent = "Enhanced review view";
  master.append(masterInput, masterText);
  root.appendChild(master);

  for (const group of GAME_REVIEW_GROUPS) {
    const groupSettings = review[group.id];
    const section = document.createElement("div");
    section.className = "game-review-group";
    section.dataset.gamePluginReviewGroup = plugin.id;
    section.dataset.reviewGroup = group.id;

    const head = document.createElement("label");
    head.className = "check-line game-review-group-head";
    const enabled = document.createElement("input");
    enabled.type = "checkbox";
    enabled.checked = groupSettings.enabled;
    enabled.dataset.gamePluginReviewSetting = plugin.id;
    enabled.dataset.reviewGroup = group.id;
    enabled.dataset.reviewKey = "enabled";
    enabled.addEventListener("change", () => updateGamePluginReviewSetting(plugin));
    const title = document.createElement("strong");
    title.textContent = group.label;
    head.append(enabled, title);

    const options = document.createElement("div");
    options.className = "game-review-options";
    for (const [key, labelText] of group.options) {
      options.appendChild(renderReviewCheckbox(
        plugin,
        group.id,
        key,
        labelText,
        groupSettings[key],
      ));
    }

    section.append(head, options);
    root.appendChild(section);
  }
  return root;
}

function syncGamePluginCatalog(nextPlugins) {
  gamePlugins = Array.isArray(nextPlugins) ? nextPlugins : [];
  renderGamePlugins();
  updateGameDetectionStatus();
  if (clipsCache.length) renderClips();
  if (currentClip) {
    renderGameEventRail(currentClip);
    renderGameMetadataPanel(currentClip);
  }
}

function renderGamePlugins() {
  const root = $("supported-games");
  root.replaceChildren();
  if (!gamePlugins.length) {
    const empty = document.createElement("div");
    empty.className = "hint";
    empty.textContent = "no supported games available";
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

    row.append(
      enabled,
      icon,
      meta,
      renderGamePluginModeControl(plugin, settings),
      renderGamePluginReviewControls(plugin, settings)
    );
    root.appendChild(row);
    syncGamePluginReviewControls(plugin);
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

function captureSettingsValue(settings = settingsFormSource()) {
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
  syncCaptureBackendSummary();
  updateCaptureStatus();
}

function syncCaptureBackendSummary() {
  const summary = $("backend-summary");
  if (!summary) return;
  if ($("set-backend").value === "desktop_duplication") {
    summary.textContent =
      "Removes the Windows 10 capture border for displays and regions. Display/region only (not single windows); the mouse cursor may be missing on some systems. Falls back to Windows Graphics Capture if unavailable.";
  } else {
    summary.textContent =
      "Windows Graphics Capture works everywhere, including single windows. On Windows 10 it may show a yellow capture border.";
  }
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
  const audio = settingsFormSource().audio || defaultAudioSettings();
  fillDeviceSelect("set-output-device", audioDevices.outputs, "Default output device", audio.output_device_id);
  fillDeviceSelect("set-mic-device", audioDevices.inputs, "Default microphone", audio.mic_device_id);
}

function renderVideoEncoderSelect() {
  const select = $("set-encoder");
  const selected = settingsFormSource().video_encoder || "auto";
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
  $("set-audio-split-output").disabled = !outputEnabled;
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
  $("rail-status").classList.toggle("stopped", !recordingActive);
  $("rail-status").setAttribute("aria-pressed", String(recordingActive));
  $("rail-status").title = recordingActive ? "Stop recording" : `Start ${source} recording`;
  $("rail-status-text").textContent = recordingActive ? "Rec" : "Off";
  $("rail-save").disabled = !recordingActive;
  renderRailGame();
}

function saveHotkeyLabel() {
  return (currentSettings && currentSettings.hotkey) || $("set-hotkey").value || "Alt+F10";
}

function updateHotkeyLabels(hotkey = saveHotkeyLabel()) {
  const label = String(hotkey || "Alt+F10");
  $("rail-hotkey").textContent = label;
  $("rail-hotkey").title = `Save Replay: ${label}`;
  $("rail-save").title = `Save Replay (${label})`;
}

function fallbackCaptureSourceLabel(settings) {
  if (settings && settings.capture_mode === "display_region") {
    const display = displays.find((item) => isFullDisplayRegion(settings.capture_region, item));
    if (display) return `Display: ${display.name}`;
  }
  return captureSourceLabel(settings);
}


async function toggleRecording() {
  const next = !recordingActive;
  $("rail-status").disabled = true;
  try {
    recordingActive = await invoke("set_recording", { recording: next });
    updateCaptureStatus();
  } catch (e) {
    $("error").textContent = e;
  } finally {
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
  setHotkeyStatus("Press an F-key, mouse button, or Ctrl/Alt/Shift plus a keyboard key.", "recording");
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

  applyHotkeyCaptureResult(hotkeyFromKeyEvent(ev));
}

function recordMouseHotkey(ev) {
  if (!hotkeyCaptureActive) {
    if (ev.button === 0) return;
    beginHotkeyCapture();
  }
  if (ev.button === 0) return;
  ev.preventDefault();
  ev.stopPropagation();

  applyHotkeyCaptureResult(hotkeyFromMouseEvent(ev));
}

function applyHotkeyCaptureResult(result) {
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
    displaysLoaded = true;
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

async function ensureDisplaysLoaded() {
  if (displaysLoaded) return;
  if (!displaysLoadPromise) {
    displaysLoadPromise = loadDisplays().finally(() => {
      displaysLoadPromise = null;
    });
  }
  await displaysLoadPromise;
}

async function loadAudioDevices() {
  try {
    audioDevices = await invoke("list_audio_devices");
    audioDevicesLoaded = true;
    renderAudioDeviceSelects();
  } catch (e) {
    $("error").textContent = e;
  }
}

async function ensureAudioDevicesLoaded() {
  if (audioDevicesLoaded) return;
  if (!audioDevicesLoadPromise) {
    audioDevicesLoadPromise = loadAudioDevices().finally(() => {
      audioDevicesLoadPromise = null;
    });
  }
  await audioDevicesLoadPromise;
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
    videoEncodersLoaded = true;
  } catch (e) {
    videoEncoders = [];
    $("error").textContent = e;
  }
  renderVideoEncoderSelect();
  if (currentSettings) syncRecordingFields();
}

async function ensureVideoEncodersLoaded() {
  if (videoEncodersLoaded) return;
  if (!videoEncodersLoadPromise) {
    videoEncodersLoadPromise = loadVideoEncoders().finally(() => {
      videoEncodersLoadPromise = null;
    });
  }
  await videoEncodersLoadPromise;
}

async function loadGamePlugins() {
  try {
    syncGamePluginCatalog(await invoke("list_game_plugins"));
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
      $("game-detection-status").textContent = "Enable a supported game or add a running game window, then save.";
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
  hideClipContextMenu();
  regionMenuDisplayId = displayId || (activeDisplay() && activeDisplay().id);
  renderDisplayMenu();
  const menu = $("capture-region-menu");
  menu.hidden = false;
  positionContextMenu(menu, ev.clientX, ev.clientY);
}

function hideRegionMenu() {
  $("capture-region-menu").hidden = true;
  regionMenuDisplayId = null;
}

function positionContextMenu(menu, x, y) {
  menu.style.left = "0px";
  menu.style.top = "0px";
  const width = menu.offsetWidth || 160;
  const height = menu.offsetHeight || 80;
  const left = Math.min(Math.max(6, x), Math.max(6, window.innerWidth - width - 6));
  const top = Math.min(Math.max(6, y), Math.max(6, window.innerHeight - height - 6));
  menu.style.left = `${left}px`;
  menu.style.top = `${top}px`;
}
