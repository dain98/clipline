param(
  [int]$Limit = 100,
  [int]$Ceiling = 500,
  [string]$Mode = "osu"
)

$ErrorActionPreference = "Stop"

$clientId = $env:OSU_CLIENT_ID
$clientSecret = $env:OSU_CLIENT_SECRET
$user = if ($env:OSU_USER_ID) { $env:OSU_USER_ID } else { $env:OSU_USERNAME }

if (-not $clientId -or -not $clientSecret -or -not $user) {
  throw "Set OSU_CLIENT_ID, OSU_CLIENT_SECRET, and OSU_USER_ID or OSU_USERNAME before running this spike."
}

$tokenResponse = Invoke-RestMethod `
  -Method Post `
  -Uri "https://osu.ppy.sh/oauth/token" `
  -ContentType "application/json" `
  -Body (@{
    client_id = [int]$clientId
    client_secret = $clientSecret
    grant_type = "client_credentials"
    scope = "public"
  } | ConvertTo-Json)

$headers = @{ Authorization = "Bearer $($tokenResponse.access_token)" }
$scores = @()

for ($offset = 0; $offset -lt $Ceiling; $offset += $Limit) {
  $uri = "https://osu.ppy.sh/api/v2/users/$user/scores/recent?include_fails=1&legacy_only=0&mode=$Mode&limit=$Limit&offset=$offset"
  $page = Invoke-RestMethod -Method Get -Uri $uri -Headers $headers
  if (-not $page -or $page.Count -eq 0) {
    break
  }
  $scores += $page
  if ($page.Count -lt $Limit) {
    break
  }
}

$sanitized = $scores | ForEach-Object {
  [pscustomobject]@{
    id = $_.id
    passed = $_.passed
    rank = $_.rank
    accuracy = $_.accuracy
    pp = $_.pp
    started_at = $_.started_at
    ended_at = $_.ended_at
    total_score = $_.total_score
    max_combo = $_.max_combo
    mods = $_.mods
    beatmap = [pscustomobject]@{
      id = $_.beatmap.id
      total_length = $_.beatmap.total_length
      version = $_.beatmap.version
    }
    beatmapset = [pscustomobject]@{
      id = $_.beatmapset.id
      artist = $_.beatmapset.artist
      title = $_.beatmapset.title
      creator = $_.beatmapset.creator
    }
  }
}

$outDir = Join-Path (Get-Location) "target/osu-api-spike"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$outPath = Join-Path $outDir "recent-scores.sanitized.json"
$sanitized | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 -Path $outPath

$failed = @($sanitized | Where-Object { $_.passed -eq $false }).Count
$missingStarted = @($sanitized | Where-Object { -not $_.started_at }).Count
$missingEnded = @($sanitized | Where-Object { -not $_.ended_at }).Count
$hitCeiling = $scores.Count -ge $Ceiling

[pscustomobject]@{
  user = $user
  mode = $Mode
  scores = $scores.Count
  failed_scores = $failed
  missing_started_at = $missingStarted
  missing_ended_at = $missingEnded
  hit_ceiling = $hitCeiling
  output = $outPath
}
