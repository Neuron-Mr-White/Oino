# Oino installer for Windows (PowerShell 5.1+ / 7+).
#
# Installs oino.exe and oino-kanboard.exe into $env:OINO_PREFIX\bin (default
# $HOME\.local\bin). Every provider and unipi extension is bundled inside oino.exe.
#
# 1. Prefer a tagged release (release-manifest.json, sha256-verified).
# 2. Fall back to a source build: `cargo xtask wasm`, then
#    `cargo build --release -p oino -p oino-kanboard`.
#
# Environment: OINO_PREFIX, OINO_REF, OINO_FROM_SOURCE=1, OINO_DIR, OINO_DRY_RUN=1.

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$DryRun = $env:OINO_DRY_RUN -eq "1"
$Prefix = if ($env:OINO_PREFIX) { $env:OINO_PREFIX } else { Join-Path $HOME ".local" }
$BinDir = Join-Path $Prefix "bin"
$RepoSlug = if ($env:OINO_REPO_SLUG) { $env:OINO_REPO_SLUG } else { "Neuron-Mr-White/Oino" }
$Repo = "https://github.com/$RepoSlug.git"
$SrcDir = if ($env:OINO_DIR) { $env:OINO_DIR } else { Join-Path $HOME ".cache\oino\source" }
$Binaries = @("oino", "oino-kanboard")
$Target = "x86_64-pc-windows-msvc"

function Say([string]$Message) { Write-Host $Message }
function Have([string]$Name) { return [bool](Get-Command $Name -ErrorAction SilentlyContinue) }
function Run([string]$Exe, [string[]]$Arguments) {
    Say "+ $Exe $($Arguments -join ' ')"
    if (-not $DryRun) {
        & $Exe @Arguments
        if ($LASTEXITCODE -ne 0) { throw "command failed ($LASTEXITCODE): $Exe $($Arguments -join ' ')" }
    }
}

function Install-Release {
    if ($DryRun -or $env:OINO_FROM_SOURCE -eq "1") { return $false }
    $ManifestUrl = if ($env:OINO_REF) {
        "https://github.com/$RepoSlug/releases/download/$($env:OINO_REF)/release-manifest.json"
    } else {
        "https://github.com/$RepoSlug/releases/latest/download/release-manifest.json"
    }
    try {
        Say "Checking Oino release manifest: $ManifestUrl"
        $Manifest = Invoke-RestMethod -Uri $ManifestUrl
    } catch {
        return $false
    }
    $Tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("oino-install-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Force -Path $Tmp | Out-Null
    foreach ($Name in $Binaries) {
        $Artifact = $Manifest.artifacts | Where-Object { $_.target -eq $Target -and $_.name -eq $Name } | Select-Object -First 1
        if (-not $Artifact) { return $false }
        $Out = Join-Path $Tmp "$Name.exe"
        Invoke-WebRequest -Uri $Artifact.url -OutFile $Out
        $Hash = (Get-FileHash -Algorithm SHA256 $Out).Hash.ToLower()
        if ($Artifact.sha256 -and $Hash -ne $Artifact.sha256.ToLower()) { throw "sha256 mismatch for $Name" }
    }
    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    foreach ($Name in $Binaries) { Copy-Item (Join-Path $Tmp "$Name.exe") (Join-Path $BinDir "$Name.exe") -Force }
    Remove-Item -Recurse -Force $Tmp
    return $true
}

function Install-Source {
    if ((Test-Path "Cargo.toml") -and (Test-Path "crates\oino-bundle")) {
        $Src = (Get-Location).Path
    } else {
        $Src = $SrcDir
        if (-not (Test-Path (Join-Path $Src ".git"))) {
            if (-not (Have "git")) { throw "git is required for the source build" }
            Run "git" @("clone", $Repo, $Src)
        }
        Run "git" @("-C", $Src, "fetch", "--all", "--tags")
        $Ref = if ($env:OINO_REF) { $env:OINO_REF } else { "v1-alpha" }
        Run "git" @("-C", $Src, "checkout", $Ref)
    }
    if (-not (Have "cargo")) { throw "cargo is missing. Install Rust from https://rustup.rs/ and rerun." }
    Push-Location $Src
    try {
        Run "cargo" @("xtask", "wasm")
        Run "cargo" @("build", "--release", "-p", "oino", "-p", "oino-kanboard")
    } finally {
        Pop-Location
    }
    if (-not $DryRun) { New-Item -ItemType Directory -Force -Path $BinDir | Out-Null }
    foreach ($Name in $Binaries) {
        $From = Join-Path $Src "target\release\$Name.exe"
        Say "+ copy $From $BinDir"
        if (-not $DryRun) { Copy-Item $From (Join-Path $BinDir "$Name.exe") -Force }
    }
}

if (Install-Release) {
    Say "Installed release binaries into $BinDir"
} else {
    Say "No release binary for $Target (or download failed); building from source."
    Install-Source
}
if (-not (($env:PATH -split ';') -contains $BinDir)) {
    Say "Add $BinDir to PATH (System Properties > Environment Variables)."
}
Say "Done. Run: oino"
