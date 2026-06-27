#requires -Version 5.1
[CmdletBinding()]
param(
    [string]$CliplineExe,
    [int]$Port = 47651,
    [string]$EvidencePath = (Join-Path (Get-Location) ("clipline-fallback-validation-{0}.json" -f (Get-Date -Format "yyyyMMdd-HHmmss"))),
    [int]$TimeoutSeconds = 45,
    [switch]$UseDebugMissingPreflight,
    [switch]$ForceFallback,
    [switch]$IncludeSaveReplay,
    [switch]$KeepRunning
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-CliplineExe {
    param([string]$Requested)

    if ($Requested) {
        $resolved = Resolve-Path -LiteralPath $Requested -ErrorAction Stop
        return $resolved.ProviderPath
    }

    $repoRoot = Split-Path -Parent $PSScriptRoot
    $candidates = @(
        (Join-Path $repoRoot "target\debug\clipline-app.exe"),
        (Join-Path $env:LOCALAPPDATA "Programs\Clipline\Clipline.exe"),
        (Join-Path $env:LOCALAPPDATA "Clipline\Clipline.exe"),
        (Join-Path $env:ProgramFiles "Clipline\Clipline.exe")
    )

    foreach ($candidate in $candidates) {
        if ($candidate -and (Test-Path -LiteralPath $candidate -PathType Leaf)) {
            return (Resolve-Path -LiteralPath $candidate).ProviderPath
        }
    }

    throw "Clipline executable was not found. Pass -CliplineExe with the installed or built executable path."
}

function Add-Check {
    param(
        [System.Collections.ArrayList]$Checks,
        [string]$Name,
        [bool]$Ok,
        [object]$Details = $null
    )

    [void]$Checks.Add([ordered]@{
        name = $Name
        ok = $Ok
        details = $Details
    })
}

function Read-TextFile {
    param([string]$Path)

    if (!(Test-Path -LiteralPath $Path -PathType Leaf)) {
        return ""
    }
    $stream = [System.IO.FileStream]::new(
        $Path,
        [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::Read,
        [System.IO.FileShare]::ReadWrite
    )
    try {
        $reader = [System.IO.StreamReader]::new($stream)
        try {
            return $reader.ReadToEnd()
        }
        finally {
            $reader.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Find-FallbackUrl {
    param([string[]]$Texts)

    foreach ($text in $Texts) {
        $match = [regex]::Match($text, "Clipline fallback client:\s*(http://127\.0\.0\.1:\d+/[A-Za-z0-9_-]+)")
        if ($match.Success) {
            return $match.Groups[1].Value
        }
    }

    foreach ($text in $Texts) {
        $match = [regex]::Match($text, "startup fallback server started .* url=(http://127\.0\.0\.1:\d+/[A-Za-z0-9_-]+)")
        if ($match.Success) {
            return $match.Groups[1].Value
        }
    }

    return $null
}

function Invoke-FallbackCommand {
    param(
        [string]$BaseUrl,
        [string]$Command,
        [object]$Body = @{}
    )

    $json = $Body | ConvertTo-Json -Depth 16 -Compress
    $response = Invoke-RestMethod -Uri "$BaseUrl/invoke/$Command" -Method Post -ContentType "application/json" -Body $json
    if (-not $response.ok) {
        throw "fallback command $Command failed: $($response.error)"
    }
    return $response.value
}

function Invoke-WebRequestNoRedirect {
    param([string]$Uri)

    try {
        return Invoke-WebRequest -Uri $Uri -UseBasicParsing -MaximumRedirection 0 -ErrorAction Stop
    }
    catch {
        $response = $_.Exception.Response
        if ($response) {
            return [pscustomobject]@{
                StatusCode = [int]$response.StatusCode
                Headers = $response.Headers
            }
        }
        throw
    }
}

function Header-Value {
    param(
        [object]$Headers,
        [string]$Name
    )

    if ($Headers -is [System.Net.WebHeaderCollection]) {
        return $Headers[$Name]
    }
    $value = $Headers[$Name]
    if ($value -is [array]) {
        return $value[0]
    }
    return $value
}

function Invoke-WebRequestFirstByte {
    param([string]$Uri)

    $Range = "bytes=0-0"
    $request = [System.Net.HttpWebRequest]::Create($Uri)
    $request.Method = "GET"
    $request.AddRange(0, 0)
    $response = $request.GetResponse()
    try {
        $stream = $response.GetResponseStream()
        try {
            $buffer = New-Object byte[] 1
            $bytesRead = $stream.Read($buffer, 0, 1)
        }
        finally {
            if ($stream) {
                $stream.Dispose()
            }
        }
        return [pscustomobject]@{
            StatusCode = [int]$response.StatusCode
            Headers = $response.Headers
            BytesRead = $bytesRead
            RequestedRange = $Range
        }
    }
    finally {
        $response.Dispose()
    }
}

function Test-FallbackMediaPlayback {
    param(
        [string]$BaseUrl,
        [object]$Clips
    )

    $clip = @($Clips) | Where-Object { $_ -and $_.path } | Select-Object -First 1
    if (!$clip) {
        return @{ skipped = "skipped: no clips available" }
    }

    $clipPath = [string]$clip.path
    $encodedPath = [System.Uri]::EscapeDataString($clipPath)
    $redirect = Invoke-WebRequestNoRedirect -Uri "$BaseUrl/media-path?path=$encodedPath"
    if ([int]$redirect.StatusCode -ne 307) {
        throw "fallback media-path returned $($redirect.StatusCode), expected 307"
    }

    $mediaUrl = [string](Header-Value $redirect.Headers "Location")
    if (!$mediaUrl) {
        throw "fallback media-path did not include a Location header"
    }
    if (!$mediaUrl.StartsWith("$BaseUrl/media/", [System.StringComparison]::Ordinal)) {
        throw "fallback media-path returned non-opaque media URL: $mediaUrl"
    }
    $clipName = [System.IO.Path]::GetFileName($clipPath)
    if ($clipName -and $mediaUrl.Contains($clipName)) {
        throw "fallback media URL leaked the clip file name"
    }

    $media = Invoke-WebRequestFirstByte -Uri $mediaUrl
    if ([int]$media.StatusCode -ne 206) {
        throw "fallback media range request returned $($media.StatusCode), expected 206"
    }
    if ([int]$media.BytesRead -ne 1) {
        throw "fallback media range response returned $($media.BytesRead) bytes, expected 1"
    }
    $contentRange = [string](Header-Value $media.Headers "Content-Range")
    if (!$contentRange.StartsWith("bytes 0-0/", [System.StringComparison]::Ordinal)) {
        throw "fallback media range response had unexpected Content-Range: $contentRange"
    }

    return @{
        clip_name = $clipName
        media_status = [int]$media.StatusCode
        requested_range = $media.RequestedRange
        content_range = $contentRange
        media_url = $mediaUrl
    }
}

function Test-FallbackEventStream {
    param(
        [string]$BaseUrl,
        [int]$TimeoutSeconds
    )

    $uri = "$BaseUrl/events"
    $request = [System.Net.HttpWebRequest]::Create($uri)
    $request.Method = "GET"
    $request.Accept = "text/event-stream"
    $request.Timeout = [Math]::Max(5000, $TimeoutSeconds * 1000)
    $request.ReadWriteTimeout = [Math]::Max(5000, $TimeoutSeconds * 1000)

    $response = $request.GetResponse()
    try {
        if ([int]$response.StatusCode -ne 200) {
            throw "fallback event stream returned $($response.StatusCode), expected 200"
        }
        $contentType = [string](Header-Value $response.Headers "Content-Type")
        if (!$contentType.StartsWith("text/event-stream", [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "fallback event stream returned unexpected Content-Type: $contentType"
        }

        $stream = $response.GetResponseStream()
        try {
            $reader = [System.IO.StreamReader]::new($stream)
            try {
                $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
                $lines = [System.Collections.Generic.List[string]]::new()
                while ((Get-Date) -lt $deadline) {
                    $line = $reader.ReadLine()
                    if ($null -eq $line) {
                        break
                    }
                    $lines.Add($line)
                    if ($line -eq ": heartbeat") {
                        return @{
                            status = [int]$response.StatusCode
                            content_type = $contentType
                            heartbeat = ": heartbeat"
                        }
                    }
                }
            }
            finally {
                $reader.Dispose()
            }
        }
        finally {
            if ($stream) {
                $stream.Dispose()
            }
        }
    }
    finally {
        $response.Dispose()
    }

    throw "fallback event stream did not emit heartbeat before timeout"
}

function Assert-TextContains {
    param(
        [string]$Text,
        [string]$Needle,
        [string]$Name
    )

    if (!$Text.Contains($Needle)) {
        throw "$Name did not contain expected text: $Needle"
    }
}

function Assert-TextNotContains {
    param(
        [string]$Text,
        [string]$Needle,
        [string]$Name
    )

    if ($Text.Contains($Needle)) {
        throw "$Name contained unexpected text: $Needle"
    }
}

function Assert-TextBefore {
    param(
        [string]$Text,
        [string]$FirstNeedle,
        [string]$SecondNeedle,
        [string]$Name
    )

    $firstIndex = $Text.IndexOf($FirstNeedle, [System.StringComparison]::Ordinal)
    $secondIndex = $Text.IndexOf($SecondNeedle, [System.StringComparison]::Ordinal)
    if ($firstIndex -lt 0 -or $secondIndex -lt 0 -or $firstIndex -gt $secondIndex) {
        throw "$Name did not contain expected order: $FirstNeedle before $SecondNeedle"
    }
}

function Wait-TextFileContains {
    param(
        [string]$Path,
        [string]$Needle,
        [string]$Name,
        [int]$TimeoutSeconds
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        $text = Read-TextFile $Path
        if ($text.Contains($Needle)) {
            return $text
        }
        Start-Sleep -Milliseconds 500
    } while ((Get-Date) -lt $deadline)

    throw "Timed out waiting for $Name to contain expected text: $Needle"
}

$checks = [System.Collections.ArrayList]::new()
$process = $null
$stdoutPath = Join-Path $env:TEMP ("clipline-fallback-validation-{0}.out.log" -f ([guid]::NewGuid().ToString("N")))
$stderrPath = Join-Path $env:TEMP ("clipline-fallback-validation-{0}.err.log" -f ([guid]::NewGuid().ToString("N")))
$diagnosticLogPath = Join-Path $env:APPDATA "Clipline\clipline.log"
$startedAt = Get-Date

try {
    $exe = Resolve-CliplineExe $CliplineExe
    Add-Check $checks "resolve executable" $true @{ path = $exe }

    if (Test-Path -LiteralPath $diagnosticLogPath -PathType Leaf) {
        Remove-Item -LiteralPath $diagnosticLogPath -Force -ErrorAction Stop
    }
    Add-Check $checks "reset diagnostic log" $true @{ path = $diagnosticLogPath }

    $args = [System.Collections.Generic.List[string]]::new()
    if ($ForceFallback) {
        $args.Add("--force-fallback-client")
    }
    if ($UseDebugMissingPreflight) {
        $args.Add("--debug-webview2-preflight")
        $args.Add("missing")
    }
    $args.Add("--fallback-port")
    $args.Add([string]$Port)

    $process = Start-Process -FilePath $exe -ArgumentList $args.ToArray() -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath -WindowStyle Hidden -PassThru
    Add-Check $checks "launch process" $true @{ pid = $process.Id; args = $args.ToArray() }

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $baseUrl = $null
    do {
        Start-Sleep -Milliseconds 500
        $stdout = Read-TextFile $stdoutPath
        $stderr = Read-TextFile $stderrPath
        $diagnostic = Read-TextFile $diagnosticLogPath
        $baseUrl = Find-FallbackUrl @($stdout, $stderr, $diagnostic)
    } while (!$baseUrl -and (Get-Date) -lt $deadline)

    if (!$baseUrl) {
        throw "Timed out waiting for Clipline fallback URL. stderr: $(Read-TextFile $stderrPath)"
    }
    Add-Check $checks "discover fallback URL" $true @{ url = $baseUrl }

    $diagnosticText = Wait-TextFileContains $diagnosticLogPath "fallback frontend_ready received" "diagnostic log" $TimeoutSeconds
    Assert-TextContains $diagnosticText "setup start launched_by_autostart=" "diagnostic log"
    Assert-TextContains $diagnosticText "startup fallback server started" "diagnostic log"
    Assert-TextContains $diagnosticText "native save hotkey initialized" "diagnostic log"
    Assert-TextContains $diagnosticText "fallback frontend_ready received" "diagnostic log"
    Assert-TextContains $diagnosticText "webviews=[]" "diagnostic log"
    Assert-TextBefore $diagnosticText "setup start launched_by_autostart=" "startup fallback server started" "diagnostic log"
    Assert-TextBefore $diagnosticText "native save hotkey initialized" "startup fallback server started" "diagnostic log"
    Assert-TextBefore $diagnosticText "startup fallback server started" "fallback frontend_ready received" "diagnostic log"
    Assert-TextNotContains $diagnosticText "normal launch opening main window" "diagnostic log"
    Assert-TextNotContains $diagnosticText "open_main_window start" "diagnostic log"
    Add-Check $checks "fallback starts before WebView creation" $true @{ log = $diagnosticLogPath }
    Add-Check $checks "fallback native save hotkey available" $true @{ log = $diagnosticLogPath }
    Add-Check $checks "fallback browser frontend_ready" $true @{ log = $diagnosticLogPath }

    if ($UseDebugMissingPreflight) {
        Assert-TextContains $diagnosticText "debug WebView2 preflight override applied" "diagnostic log"
        Assert-TextContains $diagnosticText "effective_preflight=Missing" "diagnostic log"
        Add-Check $checks "debug missing WebView2 preflight selected fallback" $true $null
    } elseif (!$ForceFallback) {
        Assert-TextContains $diagnosticText "startup fallback launch requested webview_preflight=Missing" "diagnostic log"
        Add-Check $checks "real missing WebView2 preflight selected fallback" $true $null
    }

    $index = Invoke-WebRequest -Uri $baseUrl -UseBasicParsing
    Assert-TextContains $index.Content "__CLIPLINE_FALLBACK__" "fallback index"
    Assert-TextContains $index.Content "client-bridge.js" "fallback index"
    Add-Check $checks "fallback shared UI served" $true @{ status = [int]$index.StatusCode }

    $commandEndpoints = @(
        "/invoke/get_settings",
        "/invoke/list_clips",
        "/invoke/storage_status",
        "/invoke/list_game_plugins",
        "/invoke/memory_status"
    )
    $commandResults = @{}
    $clips = $null
    foreach ($endpoint in $commandEndpoints) {
        $command = ($endpoint -split "/")[-1]
        $value = Invoke-FallbackCommand -BaseUrl $baseUrl -Command $command
        if ($command -eq "list_clips") {
            $clips = $value
        }
        $commandResults[$command] = if ($null -eq $value) { "null" } else { $value.GetType().FullName }
    }
    Add-Check $checks "fallback invoke smoke" $true $commandResults

    $mediaDetails = Test-FallbackMediaPlayback -BaseUrl $baseUrl -Clips $clips
    Add-Check $checks "fallback media playback smoke" $true $mediaDetails

    $eventStreamDetails = Test-FallbackEventStream -BaseUrl $baseUrl -TimeoutSeconds $TimeoutSeconds
    Add-Check $checks "fallback event stream smoke" $true $eventStreamDetails

    if ($IncludeSaveReplay) {
        Invoke-FallbackCommand -BaseUrl $baseUrl -Command "save_replay" | Out-Null
        Add-Check $checks "optional save_replay command" $true $null
    }

    $evidence = [ordered]@{
        ok = $true
        started_at = $startedAt.ToString("o")
        finished_at = (Get-Date).ToString("o")
        clipline_exe = $exe
        base_url = $baseUrl
        port = $Port
        stdout_path = $stdoutPath
        stderr_path = $stderrPath
        diagnostic_log_path = $diagnosticLogPath
        checks = $checks
    }
    $evidence | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $EvidencePath -Encoding UTF8
    Write-Host "Clipline fallback validation passed. Evidence: $EvidencePath"
}
catch {
    Add-Check $checks "validation failure" $false @{ error = $_.Exception.Message }
    $evidence = [ordered]@{
        ok = $false
        started_at = $startedAt.ToString("o")
        finished_at = (Get-Date).ToString("o")
        evidence_path = $EvidencePath
        stdout_path = $stdoutPath
        stderr_path = $stderrPath
        diagnostic_log_path = $diagnosticLogPath
        checks = $checks
    }
    $evidence | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $EvidencePath -Encoding UTF8
    Write-Error -ErrorAction Continue "Clipline fallback validation failed. Evidence: $EvidencePath. $($_.Exception.Message)"
    exit 1
}
finally {
    if ($process -and !$KeepRunning) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    }
}
