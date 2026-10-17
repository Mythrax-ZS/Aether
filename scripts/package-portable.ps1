<#
.SYNOPSIS
  Builds the portable edition of Aether: a zip with the program and a `portable.flag` marker, so
  everything Aether writes lives in an `AetherData` folder next to the program.

.DESCRIPTION
  Runs a release build without installers (`tauri build --no-bundle`), stages the files, zips them
  into `dist\Aether-<version>-portable-x64.zip` and prints the SHA-256 of the archive.

.PARAMETER SkipBuild
  Reuse the existing `target\release\aether.exe` instead of building.

.PARAMETER OutDir
  Where to put the zip. Defaults to `dist` in the repository.

.EXAMPLE
  pwsh scripts/package-portable.ps1
#>
[CmdletBinding()]
param(
  [switch]$SkipBuild,
  [string]$OutDir
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not $OutDir) { $OutDir = Join-Path $root 'dist' }

$conf = Get-Content (Join-Path $root 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
$version = $conf.version
$exe = Join-Path $root 'target\release\aether.exe'

if (-not $SkipBuild) {
  Write-Host "> Building Aether $version (release, no installers)..." -ForegroundColor Cyan
  Push-Location $root
  try {
    npx tauri build --no-bundle
    if ($LASTEXITCODE -ne 0) { throw "tauri build failed ($LASTEXITCODE)" }
  } finally { Pop-Location }
}
if (-not (Test-Path $exe)) { throw "Missing $exe - run without -SkipBuild first." }

$name = "Aether-$version-portable-x64"
$stage = Join-Path $OutDir $name
$zip = Join-Path $OutDir "$name.zip"
New-Item -ItemType Directory -Force $OutDir | Out-Null
Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $zip -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null

Copy-Item $exe (Join-Path $stage 'Aether.exe')
Copy-Item (Join-Path $root 'LICENSE') $stage

# The marker is what switches Aether into portable mode (see crates/aether-core/src/paths.rs).
Set-Content -Path (Join-Path $stage 'portable.flag') -Encoding ascii -Value @(
  'While this file sits next to Aether.exe, Aether keeps everything it stores',
  '(hosts, settings, keys, logs) in the AetherData folder beside it.',
  'Delete this file to use the normal per-user locations instead.'
)

Set-Content -Path (Join-Path $stage 'README.txt') -Encoding ascii -Value @(
  "Aether $version - portable edition",
  '',
  'Run Aether.exe. Nothing is installed; everything Aether stores is kept in the',
  '"AetherData" folder that appears next to it, so you can move or back up this',
  'whole folder (a USB stick works).',
  '',
  'Saved passwords and passphrases are encrypted with your Windows account (DPAPI):',
  'they work for you on this PC and are useless if the folder is copied elsewhere.',
  '',
  'Requires Windows 10 or 11 (64-bit) and the Microsoft Edge WebView2 runtime, which',
  'is already part of current Windows. If Aether does not start, install it from',
  'https://developer.microsoft.com/microsoft-edge/webview2/',
  '',
  'Licence: see LICENSE.'
)

# ZipFile (not Compress-Archive): Windows PowerShell 5.1's Compress-Archive writes backslashes into entry
# names, which other unzip tools handle badly. The base directory goes in the archive so unzipping
# creates one tidy folder.
Add-Type -AssemblyName System.IO.Compression.FileSystem
[IO.Compression.ZipFile]::CreateFromDirectory($stage, $zip, [IO.Compression.CompressionLevel]::Optimal, $true)
Remove-Item $stage -Recurse -Force

$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant()
$mb = [math]::Round((Get-Item $zip).Length / 1MB, 2)
Write-Host ''
Write-Host "[ok] $zip  ($mb MB)" -ForegroundColor Green
Write-Host "  sha256 $hash"
