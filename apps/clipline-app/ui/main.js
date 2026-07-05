// Bootstrap: backend events, DOM wiring, and app startup.

// Custom window chrome — registered last so handlers in review-player.js exist.
$('win-min').addEventListener('click', async () => {
  try {
    await invoke('minimize_main_window');
  } catch (e) {
    $('error').textContent = e;
  }
});
$('win-max').addEventListener('click', () => appWindow.toggleMaximize());
$('win-close').addEventListener('click', requestWindowClose);
/* ---- backend events ---- */

listen("status", (e) => {
  const s = e.payload;
  recordingActive = s.recording;
  fullSessionRecordingActive = Boolean(s.full_session);
  $("rail-dot").className = "dot" + (s.recording ? " on" : "");
  updateCaptureStatus();
});

listen("saved", (e) => {
  $("error").textContent = "";
  const s = e.payload;
  const savedKind = s.full_session ? "session" : "replay";
  setNotice(s.gc_deleted
    ? `cleaned up ${s.gc_deleted} old clip${s.gc_deleted > 1 ? "s" : ""} (${fmtBytes(s.gc_freed_bytes)})`
    : `saved ${fmtDur(s.seconds)} ${savedKind}`, { transient: true });
  refresh();
});

listen("osu-enrichment-updated", () => {
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

listen("suspend-review-playback", () => suspendReviewPlayback());

listen("game-detection", (e) => {
  activeDetectedGame = e.payload || null;
  updateCaptureStatus();
  updateGameDetectionStatus();
});

listen("cloud-upload-progress", (e) => {
  const progress = e.payload || {};
  upsertCloudProgress(progress);
  if (progress.error) {
    $("error").textContent = progress.error;
  } else if (progress.upload_status === "uploading") {
    const total = Number(progress.file_size_bytes) || 0;
    const done = Number(progress.received_size_bytes) || 0;
    setDeckStatus(total > 0
      ? `cloud upload ${Math.round((done / total) * 100)}%`
      : "cloud upload in progress");
  } else if (progress.upload_status === "processing") {
    setDeckStatus("cloud upload processing");
  }
  renderClips();
});

/* ---- wiring ---- */

$("review-back").addEventListener("click", () => closeReview());

// Gallery (library home) controls.
$("gallery-search").addEventListener("input", (ev) => {
  gallerySearch = ev.target.value.trim().toLowerCase();
  renderClips();
});
$("gallery-source-tabs").addEventListener("click", (ev) => {
  const tab = ev.target.closest(".source-tab");
  if (!tab) return;
  gallerySource = tab.dataset.gallerySource === "cloud" ? "cloud" : "local";
  if (gallerySource === "cloud") exitSelectMode();
  renderClips();
  if (gallerySource === "cloud") loadCloudClips({ force: true });
});
$("gallery-select-toggle").addEventListener("click", () => {
  selectMode = !selectMode;
  if (!selectMode) clearSelection();
  syncSelectionControls();
});
$("bulk-select-all").addEventListener("click", selectAllVisible);
$("bulk-clear").addEventListener("click", clearSelection);
$("bulk-cancel").addEventListener("click", exitSelectMode);
$("bulk-delete").addEventListener("click", bulkDeleteSelected);
$("gallery-sort").addEventListener("change", (ev) => { gallerySort = ev.target.value; renderClips(); });
$("gallery-group").addEventListener("change", (ev) => { galleryGroup = ev.target.value; renderClips(); });
$("gallery-filter").addEventListener("click", (ev) => {
  const chip = ev.target.closest(".g-chip");
  if (!chip) return;
  galleryFilter = chip.dataset.filter;
  for (const c of $("gallery-filter").querySelectorAll(".g-chip")) c.classList.toggle("on", c === chip);
  renderClips();
});
$("rail-status").addEventListener("click", toggleRecording);
$("set-capture").addEventListener("change", () => {
  captureTargetDirty = true;
  syncCaptureFields();
});
$("set-backend").addEventListener("change", syncCaptureBackendSummary);
$("set-theme").addEventListener("change", () => applyUiTheme($("set-theme").value));
for (const id of ["set-output-enabled", "set-audio-split-output", "set-mic-enabled"]) {
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
$("detect-games").addEventListener("click", showDetectedGamesDialog);
$("add-detected-games").addEventListener("click", addSelectedDetectedGames);
$("cancel-detected-games").addEventListener("click", hideDetectedGamesDialog);
$("detected-games-dialog").addEventListener("close", resetDetectedGamesDialog);
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
$("cloud-host-url").addEventListener("input", syncCloudHttpWarning);
$("cloud-host-url").addEventListener("change", syncCloudHttpWarning);
$("cloud-connect").addEventListener("click", connectCloud);
$("cloud-disconnect").addEventListener("click", disconnectCloud);
$("set-games-auto-detect").addEventListener("change", updateGameDetectionStatus);
for (const id of [
  "set-buffer",
  "set-replay",
  "set-encoder",
  "set-output-resolution",
  "set-bitrate",
  "set-fps",
  "recording-mode-basic",
  "recording-mode-advanced",
  "set-output-width",
  "set-output-height",
  "set-custom-bitrate",
  "set-custom-fps",
]) {
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
  if (!$("clip-context-menu").contains(ev.target)) hideClipContextMenu();
});
document.addEventListener("contextmenu", (ev) => {
  ev.preventDefault();
  hideRegionMenu();
  hideClipContextMenu();
});
$("clip-context-menu").addEventListener("contextmenu", (ev) => ev.preventDefault());
$("clip-menu-play").addEventListener("click", () => {
  const entry = cloudContextTarget;
  hideClipContextMenu();
  if (entry) openCloudEntryInApp(entry);
});
$("clip-menu-open-cloud-page").addEventListener("click", () => {
  const entry = cloudContextTarget;
  hideClipContextMenu();
  if (entry) openCloudClipUrl(entry);
});
$("clip-menu-copy-cloud-link").addEventListener("click", () => {
  const entry = cloudContextTarget;
  hideClipContextMenu();
  if (entry) copyCloudUrl(entry);
});
$("clip-menu-export-play").addEventListener("click", () => {
  const target = gamePlayContextTarget;
  hideClipContextMenu();
  if (target) {
    gamePlayContextTarget = target;
    exportPlayClip().finally(() => {
      if (gamePlayContextTarget === target) gamePlayContextTarget = null;
    });
  }
});
$("clip-menu-upload").addEventListener("click", () => {
  const clip = clipContextTarget;
  const record = clipContextRecord();
  hideClipContextMenu();
  if (!clip) return;
  const uploaded = record && record.remote_url && record.upload_status.startsWith("uploaded_");
  if (uploaded) copyCloudUrl(record);
  else openUploadDialog(clip);
});
$("clip-menu-rename").addEventListener("click", () => {
  const clip = clipContextTarget;
  hideClipContextMenu();
  if (clip) beginClipRename(clip);
});
$("clip-menu-rename-file").addEventListener("click", () => {
  const clip = clipContextTarget;
  hideClipContextMenu();
  if (clip) openRenameFileDialog(clip);
});
$("clip-menu-delete").addEventListener("click", () => {
  const clip = clipContextTarget;
  hideClipContextMenu();
  if (clip) deleteClip(clip.path);
});
window.addEventListener("resize", () => {
  renderRegionEditor();
  updateStageFrame();
  hideRegionMenu();
  hideClipContextMenu();
});
$("settings-save").addEventListener("click", async () => {
  $("settings-status").textContent = "";
  $("error").textContent = "";
  if (!syncSettingsDraftFromForm().hotkey) {
    setHotkeyStatus("Save Replay needs at least one keybind.", "error");
    $("settings-status").textContent = "Save Replay needs at least one keybind.";
    return;
  }
  try {
    const saved = await invoke("save_settings", { settings: syncSettingsDraftFromForm() });
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
  syncGameEventRail(video.currentTime || 0);
  syncGamePlayRail(video.currentTime || 0);
  paintTimeline();
  scheduleOverlayIdleCheck();
});
video.addEventListener("pause", () => {
  syncPlayState();
  clearOverlayIdleCheck();
  paintTimeline();
  updateOverlay();
});
video.addEventListener("timeupdate", () => {
  maybeFollow(video.currentTime || 0);
  paintTimeline();
  syncGameEventRail(video.currentTime || 0);
  syncGamePlayRail(video.currentTime || 0);
});
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
$("game-event-rail-toggle").addEventListener("click", () => {
  setGameEventRailCollapsed(!gameEventRailCollapsed);
});
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
$("rename-clip").addEventListener("click", () => beginClipRename());
$("clip-title-edit").addEventListener("submit", saveClipRename);
$("rename-cancel").addEventListener("click", cancelClipRename);
$("rename-input").addEventListener("keydown", (ev) => {
  if (ev.key !== "Escape") return;
  ev.preventDefault();
  cancelClipRename();
});
$("rename-file-save").addEventListener("click", submitRenameFileDialog);
$("rename-file-cancel").addEventListener("click", () => closeRenameFileDialog());
$("rename-file-input").addEventListener("keydown", (ev) => {
  if (ev.key === "Enter") {
    ev.preventDefault();
    submitRenameFileDialog();
  } else if (ev.key === "Escape") {
    ev.preventDefault();
    closeRenameFileDialog();
  }
});
$("upload-clip").addEventListener("click", () => {
  if (!currentClip) return;
  if (isCloudOnlyReviewClip(currentClip)) {
    copyCloudUrl({ remote_url: currentClip.cloud_remote_url || "" });
    return;
  }
  const record = clipCloudRecord(currentClip);
  const uploaded = record && record.remote_url && record.upload_status.startsWith("uploaded_");
  if (uploaded) copyCloudUrl(record);
  else openUploadDialog(currentClip);
});
$("upload-confirm").addEventListener("click", submitUploadDialog);
$("upload-cancel").addEventListener("click", closeUploadDialog);
$("upload-title").addEventListener("keydown", (ev) => {
  if (ev.key !== "Enter") return;
  ev.preventDefault();
  submitUploadDialog();
});
$("upload-dialog").addEventListener("click", (ev) => {
  if (ev.target === $("upload-dialog")) closeUploadDialog();
});

$("game-plugin-settings-close").addEventListener("click", hideGamePluginSettingsDialog);
$("game-plugin-settings-dialog").addEventListener("click", (ev) => {
  if (ev.target === $("game-plugin-settings-dialog")) hideGamePluginSettingsDialog();
});
$("game-plugin-settings-dialog").addEventListener("close", () => {
  gamePluginSettingsDialogPluginId = null;
});
document.querySelectorAll("[data-game-plugin-settings-tab]").forEach((tab) => {
  tab.addEventListener("click", () => setGamePluginSettingsTab(tab.dataset.gamePluginSettingsTab));
});

$("trim-mode-toggle").addEventListener("click", () => setSimpleTrimMode(!simpleTrimMode));
$("zoom-in").addEventListener("click", () => zoomAtPlayhead(0.5));
$("zoom-out").addEventListener("click", () => zoomAtPlayhead(2));
// Plain click frames the trim selection (the editing default); Shift-click fits
// the whole clip — mirroring \ and Shift+\.
$("zoom-fit").addEventListener("click", (ev) => (ev.shiftKey ? zoomFit() : zoomToSelection()));
$("snap-toggle").addEventListener("click", toggleSnap);

// Keyboard shortcuts guide — the corner "K" keycap opens it; click the X or the
// backdrop (or press Esc, which the modal dialog handles) to close.
$("keys-close").addEventListener("click", () => $("keys-dialog").close());
$("keys-dialog").addEventListener("click", (ev) => {
  if (ev.target === $("keys-dialog")) $("keys-dialog").close();
});

$("rail-save").addEventListener("click", () => invoke("save_replay"));
$("rail-profile").addEventListener("click", openRailProfile);
$("rail-settings").addEventListener("click", () => {
  if (settingsOpen) requestSettingsClose();
  else toggleSettings(true);
});
$("settings-close").addEventListener("click", requestSettingsClose);
$("settings-page").addEventListener("input", () => syncSettingsDraftFromForm());
$("settings-page").addEventListener("change", () => syncSettingsDraftFromForm());
$("settings-page").addEventListener("pointerdown", (ev) => {
  if (ev.target === $("settings-page")) requestSettingsClose({ allowDiscard: false });
});
for (const hotkeyFieldId of HOTKEY_FIELD_IDS) {
  const field = $(hotkeyFieldId);
  field.addEventListener("focus", () => beginHotkeyCapture(hotkeyFieldId));
  field.addEventListener("click", () => beginHotkeyCapture(hotkeyFieldId));
  field.addEventListener("keydown", (ev) => recordHotkey(hotkeyFieldId, ev));
  field.addEventListener("mousedown", (ev) => recordMouseHotkey(hotkeyFieldId, ev));
  field.addEventListener("auxclick", (ev) => ev.preventDefault());
  field.addEventListener("contextmenu", (ev) => ev.preventDefault());
  field.addEventListener("paste", (ev) => ev.preventDefault());
  field.addEventListener("blur", () => {
    if (activeHotkeyCaptureId === hotkeyFieldId) endHotkeyCapture(hotkeyFieldId, "Shortcut unchanged.");
  });
}

document.querySelectorAll("#settings-tabs .tab").forEach((tab) => {
  tab.addEventListener("click", () => {
    syncSettingsDraftFromForm();
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
  if (
    $("confirm-dialog").open ||
    $("quit-dialog").open ||
    $("update-dialog").open ||
    $("upload-dialog").open ||
    $("game-plugin-settings-dialog").open ||
    $("keys-dialog").open
  ) return; // a dialog owns the keyboard
  if (ev.code === "Escape" && settingsOpen) {
    ev.preventDefault();
    requestSettingsClose();
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
  // Gallery multi-select shortcuts (only when the library view is shown).
  const galleryVisible = !$("gallery-view").hidden;
  if (galleryVisible && !currentClip && (selectMode || selectedClipPaths.size > 0)) {
    if (ev.code === "Escape") {
      ev.preventDefault();
      if (selectedClipPaths.size > 0) clearSelection();
      else exitSelectMode();
      return;
    }
    if (ev.code === "KeyA" && ev.ctrlKey && selectMode) {
      ev.preventDefault();
      selectAllVisible();
      return;
    }
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
    case "set-in":
      if (!legacyTimelineEnabled() && !simpleTrimMode) setSimpleTrimMode(true);
      setTrim(video.currentTime || 0, trimEnd);
      break;
    case "set-out":
      if (!legacyTimelineEnabled() && !simpleTrimMode) setSimpleTrimMode(true);
      setTrim(trimStart, video.currentTime || 0);
      break;
    case "next-marker": jumpMarker(1); break;
    case "prev-marker": jumpMarker(-1); break;
    case "next-edit": jumpEdit(1); break;
    case "prev-edit": jumpEdit(-1); break;
    case "zoom":
      if (legacyTimelineEnabled() || simpleTrimMode) zoomAtPlayhead(intent.factor);
      break;
    case "zoom-fit":
      if (legacyTimelineEnabled() || simpleTrimMode) zoomFit();
      break;
    case "zoom-selection":
      if (legacyTimelineEnabled() || simpleTrimMode) zoomToSelection();
      break;
    case "toggle-snap":
      if (legacyTimelineEnabled()) toggleSnap();
      break;
    case "close": closeReview(); break;
  }
});

/* ---- boot ---- */

updateViews();
syncPlayState();
syncVolume();
syncAllRangeProgress();
function reportFrontendReady() {
  invoke("frontend_ready").catch((e) => console.warn("frontend_ready failed:", e));
}
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
afterNextPaint().then(() => {
  refresh().catch((e) => $("error").textContent = e);
  refreshMemoryUsage();
  ensureDisplaysLoaded().catch((e) => $("error").textContent = e);
  window.setTimeout(() => {
    ensureAudioDevicesLoaded().catch((e) => $("error").textContent = e);
    ensureVideoEncodersLoaded().catch((e) => $("error").textContent = e);
  }, 750);
});
reportFrontendReady();
setInterval(refreshMemoryUsage, 2000);
