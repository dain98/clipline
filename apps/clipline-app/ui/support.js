// Private diagnostic report preparation and explicit submission.
var preparedSupportReport = null;
var submittedSupportReportId = "";

function supportSetStatus(message, isError = false) {
  const node = $("support-status");
  node.textContent = String(message || "");
  node.classList.toggle("error-text", Boolean(isError));
}

function supportSetBusy(busy) {
  for (const id of ["support-prepare", "support-open-logs", "support-send", "support-save-copy", "support-discard"]) {
    $(id).disabled = Boolean(busy);
  }
}

function resetSupportPreview() {
  preparedSupportReport = null;
  $("support-preview").hidden = true;
  $("support-progress").hidden = true;
  $("support-success").hidden = true;
  $("support-preview-files").replaceChildren();
}

async function prepareSupportReport() {
  const description = $("support-description").value.trim();
  supportSetStatus("");
  supportSetBusy(true);
  $("support-success").hidden = true;
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
    $("support-preview").hidden = false;
    supportSetStatus("Review the included file list, then explicitly send or save the report.");
  } catch (error) {
    supportSetStatus(error, true);
    reportFrontendDiagnostic("error", "support_prepare_failed", error);
  } finally {
    supportSetBusy(false);
  }
}

async function sendSupportReport() {
  if (!preparedSupportReport) return;
  const token = preparedSupportReport.token;
  supportSetStatus("");
  supportSetBusy(true);
  $("support-preview").hidden = true;
  $("support-progress").hidden = false;
  try {
    const result = await invoke("submit_bug_report", { token });
    submittedSupportReportId = result.report_id;
    preparedSupportReport = null;
    $("support-progress").hidden = true;
    $("support-success").hidden = false;
    $("support-success-detail").textContent =
      `Private report ${result.report_id} was received. It expires ${new Date(result.expires_at).toLocaleString()}.`;
    $("support-description").value = "";
    supportSetStatus("Thank you. No report was posted publicly.");
  } catch (error) {
    $("support-progress").hidden = true;
    $("support-preview").hidden = false;
    supportSetStatus(`${error} You can retry or save the prepared ZIP locally.`, true);
    reportFrontendDiagnostic("warn", "support_upload_failed", error);
  } finally {
    supportSetBusy(false);
  }
}

async function cancelSupportReport() {
  if (!preparedSupportReport) return;
  try {
    await invoke("cancel_bug_report", { token: preparedSupportReport.token });
    supportSetStatus("Upload cancellation requested. The prepared report remains available.");
  } catch (error) {
    supportSetStatus(error, true);
  }
}

async function discardSupportReport() {
  if (!preparedSupportReport) return;
  supportSetBusy(true);
  try {
    await invoke("discard_bug_report", { token: preparedSupportReport.token });
    resetSupportPreview();
    supportSetStatus("Prepared report discarded.");
  } catch (error) {
    supportSetStatus(error, true);
  } finally {
    supportSetBusy(false);
  }
}

async function saveSupportReportCopy() {
  if (!preparedSupportReport) return;
  supportSetBusy(true);
  try {
    const path = await invoke("save_prepared_bug_report", { token: preparedSupportReport.token });
    supportSetStatus(`Saved a local copy to ${path}`);
  } catch (error) {
    if (String(error) !== "save cancelled") supportSetStatus(error, true);
  } finally {
    supportSetBusy(false);
  }
}

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
