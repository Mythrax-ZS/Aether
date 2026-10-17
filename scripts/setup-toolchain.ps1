<#
.SYNOPSIS
  Gets a machine ready to build Aether.

.DESCRIPTION
  Checks for the Rust toolchain and Node.js. If Node.js 20+ isn't on PATH it downloads a private,
  checksum-verified copy of the current LTS into `.tools\node` (ignored by git) - nothing is
  installed system-wide. Then it installs the JavaScript dependencies.

  Rust itself and the Visual Studio Build Tools are not downloaded: they are large, machine-wide
  installs, so the script tells you how to get them instead.

.PARAMETER ToolsDir
  Where the private Node.js goes. Defaults to `.tools` in the repository.

.PARAMETER SkipInstall
  Don't run `npm install` at the end.

.EXAMPLE
  pwsh scripts/setup-toolchain.ps1
  # then, in the same shell, if it printed a PATH line, run that line once.
#>
[CmdletBinding()]
param(
  [string]$ToolsDir,
  [switch]$SkipInstall
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'   # Invoke-WebRequest is far faster without the progress bar
# Windows PowerShell 5.1 may default to older TLS versions; nodejs.org needs 1.2+.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not $ToolsDir) { $ToolsDir = Join-Path $root '.tools' }
$MinNode = [version]'20.0.0'
$MinRust = [version]'1.89.0'

function Write-Step($text) { Write-Host "> $text" -ForegroundColor Cyan }
function Write-Ok($text) { Write-Host "  [ok] $text" -ForegroundColor Green }
function Write-Todo($text) { Write-Host "  ! $text" -ForegroundColor Yellow }

function Get-ToolVersion([string]$command, [string]$pattern) {
  $found = Get-Command $command -ErrorAction SilentlyContinue
  if (-not $found) { return $null }
  $text = (& $found.Source --version 2>$null) -join ' '
  if ($text -match $pattern) { return [version]$Matches[1] }
  return $null
}

# -- Rust ---------------------------------------------------------------------
Write-Step 'Rust'
$rust = Get-ToolVersion 'rustc' '(\d+\.\d+\.\d+)'
if (-not $rust) {
  Write-Todo 'rustc was not found. Install rustup from https://rustup.rs and choose the MSVC toolchain.'
  Write-Todo 'It needs the "Desktop development with C++" workload of the Visual Studio Build Tools.'
  $missing = $true
} elseif ($rust -lt $MinRust) {
  Write-Todo "rustc $rust is older than $MinRust. Run: rustup update stable"
  $missing = $true
} else {
  Write-Ok "rustc $rust"
}

# -- Node.js ------------------------------------------------------------------
Write-Step 'Node.js'
$privateNode = Join-Path $ToolsDir 'node'
$node = Get-ToolVersion 'node' 'v(\d+\.\d+\.\d+)'
if ($node -and $node -ge $MinNode) {
  Write-Ok "node $node (on PATH)"
} elseif (Test-Path (Join-Path $privateNode 'node.exe')) {
  $v = (& (Join-Path $privateNode 'node.exe') --version) -replace '^v', ''
  Write-Ok "node $v (private copy in $privateNode)"
  $needPath = $true
} else {
  Write-Host '  Downloading the current Node.js LTS (private copy, nothing is installed system-wide)...'
  $index = Invoke-RestMethod 'https://nodejs.org/dist/index.json'
  $lts = $index | Where-Object { $_.lts } | Select-Object -First 1
  $ver = $lts.version                                     # e.g. v22.11.0
  $zipName = "node-$ver-win-x64.zip"
  $base = "https://nodejs.org/dist/$ver"
  $tmp = Join-Path ([IO.Path]::GetTempPath()) ("aether-node-" + [guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Force $tmp | Out-Null
  try {
    $zip = Join-Path $tmp $zipName
    Invoke-WebRequest "$base/$zipName" -OutFile $zip -UseBasicParsing
    $sums = (Invoke-WebRequest "$base/SHASUMS256.txt" -UseBasicParsing).Content
    if ($sums -is [byte[]]) { $sums = [Text.Encoding]::UTF8.GetString($sums) }
    $expected = ($sums -split "`n" | Where-Object { $_ -match [regex]::Escape($zipName) } | Select-Object -First 1) -split '\s+' | Select-Object -First 1
    $actual = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    if (-not $expected -or $actual -ne $expected.ToLowerInvariant()) { throw "Checksum mismatch for $zipName - refusing to use it." }
    Expand-Archive $zip -DestinationPath $tmp -Force
    New-Item -ItemType Directory -Force $ToolsDir | Out-Null
    if (Test-Path $privateNode) { Remove-Item $privateNode -Recurse -Force }
    Move-Item (Join-Path $tmp "node-$ver-win-x64") $privateNode
  } finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
  }
  Write-Ok "node $ver in $privateNode"
  $needPath = $true
}

if ($needPath) {
  $env:PATH = "$privateNode;$env:PATH"
  Write-Host ''
  Write-Host '  To use it in a new shell, run:' -ForegroundColor DarkGray
  Write-Host "    `$env:PATH = `"$privateNode;`$env:PATH`"" -ForegroundColor DarkGray
  Write-Host ''
}

# -- WebView2 -----------------------------------------------------------------
Write-Step 'WebView2 runtime'
$wv = 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
$wvUser = 'HKCU:\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
$wvVersion = (Get-ItemProperty $wv -ErrorAction SilentlyContinue).pv
if (-not $wvVersion) { $wvVersion = (Get-ItemProperty $wvUser -ErrorAction SilentlyContinue).pv }
if ($wvVersion) { Write-Ok "WebView2 $wvVersion" }
else { Write-Todo 'WebView2 runtime not found - install it from https://developer.microsoft.com/microsoft-edge/webview2/' }

# -- JavaScript dependencies --------------------------------------------------
if (-not $SkipInstall -and -not $missing) {
  Write-Step 'npm install'
  Push-Location $root
  try {
    npm install --no-audit --no-fund
    if ($LASTEXITCODE -ne 0) { throw "npm install failed ($LASTEXITCODE)" }
  } finally { Pop-Location }
  Write-Ok 'dependencies installed'
}

Write-Host ''
if ($missing) {
  Write-Host 'Almost there - fix the items marked "!" above, then run this script again.' -ForegroundColor Yellow
  exit 1
}
Write-Host 'Ready.  npm run dev  to start Aether,  npm run build  to make installers.' -ForegroundColor Green
