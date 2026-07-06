param(
  [string]$SourceDir = (Join-Path $env:APPDATA "Clipline\ffmpeg")
)

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$destination = [System.IO.Path]::GetFullPath((Join-Path $repoRoot "apps\clipline-app\ffmpeg"))
$expectedDestination = [System.IO.Path]::GetFullPath((Join-Path $repoRoot "apps\clipline-app\ffmpeg"))
if ($destination -ne $expectedDestination) {
  throw "Refusing to stage ffmpeg outside the app resource directory: $destination"
}

$source = [System.IO.Path]::GetFullPath($SourceDir)
if ($source -eq $destination) {
  throw "SourceDir must not be the destination resource directory"
}
$sourceExe = Join-Path $source "ffmpeg.exe"
if (-not (Test-Path -LiteralPath $sourceExe -PathType Leaf)) {
  throw "ffmpeg.exe not found at $sourceExe"
}

New-Item -ItemType Directory -Force -Path $destination | Out-Null
Get-ChildItem -LiteralPath $destination -Force |
  Where-Object { $_.Name -ne "README.md" } |
  ForEach-Object { Remove-Item -LiteralPath $_.FullName -Recurse -Force }

Get-ChildItem -LiteralPath $source -Force |
  ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $destination -Recurse -Force }

Write-Host "Staged FFmpeg resource from $source to $destination"
