// DOM-free private support-report workflow.
(function (root) {
  "use strict";

  const TRANSITIONS = Object.freeze({
    idle: Object.freeze({
      prepare_started: "preparing",
      reset: "idle",
    }),
    preparing: Object.freeze({
      prepare_succeeded: "prepared",
      prepare_failed: "idle",
    }),
    prepared: Object.freeze({
      upload_started: "uploading",
      discarded: "idle",
    }),
    uploading: Object.freeze({
      upload_succeeded: "success",
      upload_failed: "prepared",
      upload_cancelled: "prepared",
    }),
    success: Object.freeze({
      prepare_started: "preparing",
      reset: "idle",
    }),
  });

  function transitionSupportPhase(phase, event) {
    const next = TRANSITIONS[phase] && TRANSITIONS[phase][event];
    if (!next) throw new Error(`invalid Support transition: ${phase} -> ${event}`);
    return next;
  }

  function supportView(phase, { uploadAvailable = false, settingsDirty = false } = {}) {
    if (!Object.prototype.hasOwnProperty.call(TRANSITIONS, phase)) {
      throw new Error(`invalid Support phase: ${phase}`);
    }
    return {
      showPreparing: phase === "preparing",
      showPreview: phase === "prepared",
      showProgress: phase === "uploading",
      showSuccess: phase === "success",
      descriptionLocked: phase === "preparing" || phase === "prepared" || phase === "uploading",
      prepareDisabled: phase !== "idle" && phase !== "success",
      sendDisabled: phase !== "prepared" || !uploadAvailable,
      settingsSaveVisible: Boolean(settingsDirty),
      settingsSaveLabel: settingsDirty ? "Save Other Changes" : "Save Settings",
    };
  }

  globalThis.SupportCore = Object.freeze({
    transition: transitionSupportPhase,
    view: supportView,
  });
})(globalThis);
