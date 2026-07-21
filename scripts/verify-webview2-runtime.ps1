param(
    [switch]$RequirePayload,
    [datetime]$AsOf = (Get-Date).Date
)

$ErrorActionPreference = 'Stop'
$workspaceRoot = Split-Path $PSScriptRoot -Parent
$appRoot = Join-Path $workspaceRoot 'apps\clipline-app'
$manifestPath = Join-Path $appRoot 'webview2-fixed-runtime.json'
$configPath = Join-Path $appRoot 'tauri.standalone.conf.json'

$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
$config = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json
$reviewedOn = [datetime]::ParseExact(
    $manifest.reviewed_on,
    'yyyy-MM-dd',
    [Globalization.CultureInfo]::InvariantCulture
)
$reviewDueOn = [datetime]::ParseExact(
    $manifest.review_due_on,
    'yyyy-MM-dd',
    [Globalization.CultureInfo]::InvariantCulture
)
if ($reviewDueOn -lt $reviewedOn) {
    throw "WebView2 review due date precedes its review date."
}
if (($reviewDueOn - $reviewedOn).TotalDays -gt $manifest.max_review_age_days) {
    throw "WebView2 review window exceeds $($manifest.max_review_age_days) days."
}
if ($AsOf.Date -gt $reviewDueOn.Date) {
    throw "WebView2 Fixed Version runtime review expired on $($manifest.review_due_on)."
}

$runtimeFolder = "Microsoft.WebView2.FixedVersionRuntime.$($manifest.version).$($manifest.architecture)"
$expectedResource = "webview2-fixed/$runtimeFolder/**/*"
$expectedRuntimePath = "./webview2-fixed/$runtimeFolder"
$resources = @($config.bundle.resources)
if ($resources -notcontains $expectedResource) {
    throw "Standalone resources do not contain exact runtime glob $expectedResource."
}
if ($config.bundle.windows.webviewInstallMode.type -ne 'fixedRuntime') {
    throw "Standalone webviewInstallMode must remain fixedRuntime."
}
if ($config.bundle.windows.webviewInstallMode.path -ne $expectedRuntimePath) {
    throw "Standalone fixedRuntime path does not match manifest: $expectedRuntimePath."
}

if ($RequirePayload) {
    $payloadRoot = Join-Path (Join-Path $appRoot 'webview2-fixed') $runtimeFolder
    $runtimeExecutable = Join-Path $payloadRoot 'msedgewebview2.exe'
    if (-not (Test-Path -LiteralPath $runtimeExecutable -PathType Leaf)) {
        throw "Staged WebView2 payload is missing $runtimeExecutable."
    }
}

Write-Host "WebView2 Fixed Version $($manifest.version) $($manifest.architecture) review and config are current."
