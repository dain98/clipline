// Private diagnostic report preparation and explicit submission.
var preparedSupportReport = null;
var submittedSupportReportId = "";
var supportPhase = "idle";
var supportUploadAvailable = false;
var supportActionBusy = false;

function supportDescriptionLength() {
  const value = $("support-description").value;
  return [...value.trim()].length;
}

function supportDescriptionIsValid() {
  const length = supportDescriptionLength();
  return length >= 10 && length <= 4000;
}

function supportSetStatus(message, isError = false) {
  const node = $("support-status");
  node.textContent = String(message || "");
  node.classList.toggle("error-text", Boolean(isError));
}

function renderSupportState() {
  const view = SupportCore.view(supportPhase, {
    uploadAvailable: supportUploadAvailable,
    settingsDirty: settingsHaveUnsavedChanges(),
  });
  const description = $("support-description");
  const descriptionLength = supportDescriptionLength();
  $("support-description-count").textContent = `${descriptionLength.toLocaleString()} / 4,000`;
  description.readOnly = view.descriptionLocked;
  description.setAttribute("aria-disabled", String(view.descriptionLocked));
  $("support-preparing").hidden = !view.showPreparing;
  $("support-preview").hidden = !view.showPreview;
  $("support-progress").hidden = !view.showProgress;
  $("support-success").hidden = !view.showSuccess;
  $("support-prepare").disabled =
    supportActionBusy || view.prepareDisabled || !supportDescriptionIsValid();
  $("support-send").disabled = supportActionBusy || view.sendDisabled;
  $("support-save-copy").disabled = supportActionBusy || !view.showPreview;
  $("support-discard").disabled = supportActionBusy || !view.showPreview;
  $("support-cancel").disabled = supportActionBusy || !view.showProgress;
  $("support-open-logs").disabled = supportActionBusy;
  syncSettingsFooterForTab();
}

function transitionSupport(event) {
  supportPhase = SupportCore.transition(supportPhase, event);
  renderSupportState();
  focusSupportPhase();
}

function focusSupportPhase() {
  const targetId = {
    preparing: "support-preparing",
    prepared: "support-preview",
    uploading: "support-progress",
    success: "support-success",
  }[supportPhase];
  const target = targetId ? $(targetId) : null;
  if (target && !target.hidden) {
    target.focus({ preventScroll: true });
  }
}

function resetSupportPreview() {
  preparedSupportReport = null;
  submittedSupportReportId = "";
  $("support-preview-files").replaceChildren();
  if (supportPhase === "prepared") transitionSupport("discarded");
  else renderSupportState();
}

async function prepareSupportReport() {
  if (!supportDescriptionIsValid()) {
    supportSetStatus("Describe the problem in at least 10 and at most 4,000 characters.", true);
    return;
  }
  const description = $("support-description").value;
  supportSetStatus("");
  transitionSupport("prepare_started");
  try {
    preparedSupportReport = await invoke("prepare_bug_report", { description });
    $("support-preview-summary").textContent =
      `${preparedSupportReport.files.length} files · ${fmtBytes(preparedSupportReport.compressed_bytes)} · prepared report ${preparedSupportReport.submission_id}`;
    $("support-preview-files").replaceChildren(
      ...preparedSupportReport.files.map((file) => {
        const item = document.createElement("li");
        item.textContent = file;
        return item;
      }),
    );
    transitionSupport("prepare_succeeded");
    supportSetStatus("Review the included file list, then explicitly send or save the report.");
  } catch (error) {
    transitionSupport("prepare_failed");
    supportSetStatus(error, true);
    reportFrontendDiagnostic("error", "support_prepare_failed", error);
  }
}

async function sendSupportReport() {
  if (!preparedSupportReport || !supportUploadAvailable || supportPhase !== "prepared") return;
  const token = preparedSupportReport.token;
  supportSetStatus("");
  transitionSupport("upload_started");
  try {
    const result = await invoke("submit_bug_report", { token });
    submittedSupportReportId = result.report_id;
    preparedSupportReport = null;
    $("support-success-detail").textContent =
      `Private report ${result.report_id} was received. It expires ${new Date(result.expires_at).toLocaleString()}.`;
    $("support-description").value = "";
    transitionSupport("upload_succeeded");
    supportSetStatus("Thank you. No report was posted publicly.");
  } catch (error) {
    transitionSupport(String(error).includes("cancelled") ? "upload_cancelled" : "upload_failed");
    supportSetStatus(`${error} You can retry or save the prepared ZIP locally.`, true);
    reportFrontendDiagnostic("warn", "support_upload_failed", error);
  }
}

async function cancelSupportReport() {
  if (!preparedSupportReport || supportPhase !== "uploading") return;
  try {
    await invoke("cancel_bug_report", { token: preparedSupportReport.token });
    supportSetStatus("Upload cancellation requested. The prepared report will remain available.");
  } catch (error) {
    supportSetStatus(error, true);
  }
}

async function discardSupportReport() {
  if (!preparedSupportReport || supportPhase !== "prepared") return;
  supportActionBusy = true;
  renderSupportState();
  try {
    await invoke("discard_bug_report", { token: preparedSupportReport.token });
    resetSupportPreview();
    supportSetStatus("Prepared report discarded.");
  } catch (error) {
    supportSetStatus(error, true);
  } finally {
    supportActionBusy = false;
    renderSupportState();
  }
}

async function saveSupportReportCopy() {
  if (!preparedSupportReport || supportPhase !== "prepared") return;
  supportActionBusy = true;
  renderSupportState();
  try {
    const path = await invoke("save_prepared_bug_report", { token: preparedSupportReport.token });
    supportSetStatus(`Saved a local copy to ${path}`);
  } catch (error) {
    if (String(error) !== "save cancelled") supportSetStatus(error, true);
  } finally {
    supportActionBusy = false;
    renderSupportState();
  }
}

$("support-description").addEventListener("input", () => {
  if (supportPhase === "success") transitionSupport("reset");
  else renderSupportState();
  if (supportDescriptionIsValid() && $("support-status").classList.contains("error-text")) {
    supportSetStatus("");
  }
});
$("support-prepare").addEventListener("click", prepareSupportReport);
$("support-send").addEventListener("click", sendSupportReport);
$("support-cancel").addEventListener("click", cancelSupportReport);
$("support-discard").addEventListener("click", discardSupportReport);
$("support-save-copy").addEventListener("click", saveSupportReportCopy);
$("support-open-logs").addEventListener("click", async () => {
  try {
    await invoke("open_diagnostics_folder");
    supportSetStatus("Opened the diagnostics folder.");
  } catch (error) {
    supportSetStatus(error, true);
  }
});
$("support-copy-id").addEventListener("click", async () => {
  if (!submittedSupportReportId) return;
  try {
    await navigator.clipboard.writeText(submittedSupportReportId);
    supportSetStatus("Report ID copied.");
  } catch (error) {
    supportSetStatus(`Could not copy automatically. Report ID: ${submittedSupportReportId}`, true);
  }
});

invoke("diagnostics_location")
  .then((path) => {
    $("support-diagnostics-location").textContent = `Diagnostics folder: ${path}`;
  })
  .catch((error) => {
    $("support-diagnostics-location").textContent = `Diagnostics folder unavailable: ${error}`;
  });

invoke("support_capabilities")
  .then((capabilities) => {
    supportUploadAvailable = capabilities.upload_available === true;
    $("support-upload-availability").hidden = supportUploadAvailable;
    $("support-upload-availability").textContent = supportUploadAvailable
      ? ""
      : "Private upload is not configured in this development build. You can still prepare and save a sanitized ZIP.";
    renderSupportState();
  })
  .catch((error) => {
    supportUploadAvailable = false;
    $("support-upload-availability").hidden = false;
    $("support-upload-availability").textContent =
      `Private upload availability could not be checked: ${error}. Local preparation and save remain available.`;
    renderSupportState();
  });

renderSupportState();
