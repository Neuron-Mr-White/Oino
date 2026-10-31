# Oino installer for Windows (PowerShell 5.1+ / 7+).
#
# Default: install oino.exe from the latest GitHub release into
# %LOCALAPPDATA%\oino\bin (override: OINO_PREFIX), together with the extension
# packages under %LOCALAPPDATA%\oino\bin\extensions, and add the dir to the
# per-user PATH.
#
#   -App   install the UniPi desktop app instead: run the NSIS installer
#          silently (per-user, no admin).
#
# Every download is verified against the release's SHA256SUMS. Without a
# matching release binary the CLI falls back to a source build.
#
# Environment: OINO_INSTALL_BASE_URL (http(s):// or file:///), OINO_PREFIX,
# OINO_REF, OINO_FROM_SOURCE=1, OINO_DIR, OINO_DRY_RUN=1.

param(
    [switch]$App,
    [switch]$Deb
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$ProgressPreference = "SilentlyContinue"

$DryRun = $env:OINO_DRY_RUN -eq "1"
$RepoSlug = if ($env:OINO_REPO_SLUG) { $env:OINO_REPO_SLUG } else { "Neuron-Mr-White/Oino" }
$Repo = "https://github.com/$RepoSlug.git"
$SrcDir = if ($env:OINO_DIR) { $env:OINO_DIR } else { Join-Path $HOME ".cache\oino\source" }
$Target = "x86_64-pc-windows-msvc"

# Asset base: a custom OINO_INSTALL_BASE_URL (tests point it at a local dir),
# else the release tag, else the `latest` permalink.
$BaseUrl = if ($env:OINO_INSTALL_BASE_URL) {
    $env:OINO_INSTALL_BASE_URL
} elseif ($env:OINO_REF) {
    "https://github.com/$RepoSlug/releases/download/$($env:OINO_REF)"
} else {
    "https://github.com/$RepoSlug/releases/latest/download"
}

# CLI layout: %LOCALAPPDATA%\oino\{bin\oino.exe, bin\extensions} — `extensions`
# next to the exe is one of the loader's share-dir candidates (dist.md §5).
$InstallRoot = if ($env:OINO_PREFIX) { $env:OINO_PREFIX } else { Join-Path $env:LOCALAPPDATA "oino" }
$BinDir = Join-Path $InstallRoot "bin"

function Say([string]$Message) { Write-Host $Message }
function Have([string]$Name) { return [bool](Get-Command $Name -ErrorAction SilentlyContinue) }
function Run([string]$Exe, [string[]]$Arguments) {
    Say "+ $Exe $($Arguments -join ' ')"
    if (-not $DryRun) {
        & $Exe @Arguments
        if ($LASTEXITCODE -ne 0) { throw "command failed ($LASTEXITCODE): $Exe $($Arguments -join ' ')" }
    }
}

function Save-Url([string]$Url, [string]$Out) {
    if ($Url -match '^file://') {
        Copy-Item -LiteralPath ([Uri]$Url).LocalPath -Destination $Out -Force
    } else {
        Invoke-WebRequest -Uri $Url -OutFile $Out
    }
}

# SHA256SUMS → @{ name = hash }
$script:Sums = $null
function Get-Sums {
    if ($null -ne $script:Sums) { return $script:Sums }
    $tmp = [System.IO.Path]::GetTempFileName()
    Save-Url "$BaseUrl/SHA256SUMS" $tmp
    $map = @{}
    foreach ($line in (Get-Content $tmp)) {
        $parts = $line -split '\s+'
        if ($parts.Count -ge 2) { $map[$parts[-1]] = $parts[0] }
    }
    Remove-Item -Force $tmp
    $script:Sums = $map
    return $map
}

function Find-Asset([string]$Pattern) {
    foreach ($name in (Get-Sums).Keys) {
        if ($name -match $Pattern) { return $name }
    }
    return $null
}

function Fetch-Asset([string]$Pattern, [string]$Out) {
    $name = Find-Asset $Pattern
    if (-not $name) { return $false }
    Say "downloading $name"
    Save-Url "$BaseUrl/$name" $Out
    $hash = (Get-FileHash -Algorithm SHA256 $Out).Hash.ToLower()
    if ($hash -ne (Get-Sums)[$name].ToLower()) {
        throw "sha256 mismatch for $name — aborting"
    }
    return $true
}

function Add-BinDirToPath {
    if ($DryRun) { return }
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($null -eq $userPath) { $userPath = "" }
    if ((($userPath -split ';') -notcontains $BinDir)) {
        [Environment]::SetEnvironmentVariable("Path", "$userPath;$BinDir", "User")
        Say "Added $BinDir to the user PATH (open a new terminal)."
    }
}

function Install-Release {
    if ($DryRun) {
        Say "would install oino ($Target) into $BinDir from $BaseUrl"
        return $true
    }
    if ($env:OINO_FROM_SOURCE -eq "1") { return $false }
    $Tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("oino-install-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Force -Path $Tmp | Out-Null
    try {
        $Zip = Join-Path $Tmp "oino.zip"
        if (-not (Fetch-Asset "oino-.*-$Target\.zip$" $Zip)) { return $false }
        Say "Installing oino into $InstallRoot (bin\oino.exe + bin\extensions)"
        Expand-Archive -Path $Zip -DestinationPath $InstallRoot -Force
        if (-not (Test-Path (Join-Path $BinDir "oino.exe"))) {
            throw "archive layout changed: no bin\oino.exe in the zip"
        }
        Add-BinDirToPath
        return $true
    } finally {
        Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
    }
}

function Install-App {
    if ($DryRun) { throw "OINO_DRY_RUN does not support -App; run it for real" }
    if ($Deb) { throw "-Deb is Linux-only" }
    $Tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("oino-app-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Force -Path $Tmp | Out-Null
    try {
        $Setup = Join-Path $Tmp "UniPi-setup.exe"
        if (-not (Fetch-Asset "UniPi_.*_x64-setup\.exe$" $Setup)) {
            throw "no NSIS installer in the release (SHA256SUMS)"
        }
        Say "Running the UniPi installer silently (per-user, no admin)…"
        $p = Start-Process -FilePath $Setup -ArgumentList "/S" -Wait -PassThru
        if ($p.ExitCode -ne 0) { throw "installer exited with $($p.ExitCode)" }
        Say "Installed UniPi (Start menu → UniPi)."
    } finally {
        Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
    }
}

function Install-Source {
    if ((Test-Path "Cargo.toml") -and (Test-Path "crates\oino-core")) {
        $Src = (Get-Location).Path
    } else {
        $Src = $SrcDir
        if (-not (Test-Path (Join-Path $Src ".git"))) {
            if (-not (Have "git")) { throw "git is required for the source build" }
            Run "git" @("clone", $Repo, $Src)
        }
        Run "git" @("-C", $Src, "fetch", "--all", "--tags")
        if ($env:OINO_REF) { Run "git" @("-C", $Src, "checkout", $env:OINO_REF) }
    }
    if (-not (Have "cargo")) { throw "cargo is missing. Install Rust from https://rustup.rs/ and rerun." }
    Push-Location $Src
    try {
        Run "cargo" @("xtask", "wasm")
        Run "cargo" @("build", "--release", "-p", "oino-cli")
    } finally {
        Pop-Location
    }
    if (-not $DryRun) { New-Item -ItemType Directory -Force -Path $BinDir | Out-Null }
    $From = Join-Path $Src "target\release\oino.exe"
    Say "+ copy $From $BinDir"
    if (-not $DryRun) { Copy-Item $From (Join-Path $BinDir "oino.exe") -Force }
    # The share tree ships with releases; a source build stages the same
    # packages (fixtures excluded, dist.md §3).
    $DistShare = Join-Path $BinDir "extensions"
    if (-not $DryRun -and (Test-Path (Join-Path $Src "extensions\dist\checksums.json"))) {
        New-Item -ItemType Directory -Force -Path $DistShare | Out-Null
        Get-ChildItem (Join-Path $Src "extensions\dist") -Directory |
            Where-Object { $_.Name -notlike "oino.fixture.*" } |
            ForEach-Object {
                $dst = Join-Path $DistShare $_.Name
                if (Test-Path $dst) { Remove-Item -Recurse -Force $dst }
                Copy-Item -Recurse $_.FullName $dst
            }
    }
}

if ($App) {
    Install-App
} elseif (Install-Release) {
    Say "Done. Run: oino"
} else {
    Say "No release binary for $Target (or download failed); building from source."
    Install-Source
    Say "Done. Run: oino"
}
