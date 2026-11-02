# Oino installer for Windows (PowerShell 5.1+ / 7+).
#
# Default: install oino.exe from the latest GitHub release into
# %LOCALAPPDATA%\oino\bin (override: OINO_PREFIX), together with the extension
# packages under %LOCALAPPDATA%\oino\bin\extensions, and add the dir to the
# per-user PATH.
#
#   -App   install the Unipi desktop app instead: run the NSIS installer
#          silently (per-user, no admin).
#   -HostInstall
#          install the agent host without the desktop app (GUI-less
#          machines, sandboxes): unipi-oino-host.exe + oino.exe + the
#          bundled extensions into <prefix>\bin, plus a per-user logon
#          scheduled task "UniPi Oino host" that runs it and restarts it when
#          it fails (no admin). Safe to rerun (upgrades in place).
#            -NoService  skip the task   -NoStart  register it, don't start
#            -Uninstall  remove the task + host (data in ~\.oino\host kept)
#   -Prefix DIR   install root (same as OINO_PREFIX; default %LOCALAPPDATA%\oino)
#
# Every download is verified against the release's SHA256SUMS. Without a
# matching release binary the CLI falls back to a source build.
#
# Environment: OINO_INSTALL_BASE_URL (http(s):// or file:///), OINO_PREFIX,
# OINO_REF, OINO_FROM_SOURCE=1, OINO_DIR, OINO_DRY_RUN=1.

param(
    [switch]$App,
    [switch]$Deb,
    [switch]$HostInstall,
    [switch]$NoService,
    [switch]$NoStart,
    [switch]$Uninstall,
    [string]$Prefix
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
$InstallRoot = if ($Prefix) {
    [System.IO.Path]::GetFullPath($Prefix)
} elseif ($env:OINO_PREFIX) { $env:OINO_PREFIX } else { Join-Path $env:LOCALAPPDATA "oino" }
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
        Say "Running the Unipi installer silently (per-user, no admin)…"
        $p = Start-Process -FilePath $Setup -ArgumentList "/S" -Wait -PassThru
        if ($p.ExitCode -ne 0) { throw "installer exited with $($p.ExitCode)" }
        Say "Installed Unipi (Start menu → UniPi Oino)."
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

# ── the host without the GUI (-HostInstall) ────────────────────────────────
# Release asset unipi-oino-host-<version>-x86_64-pc-windows-msvc.zip =
#   bin\unipi-oino-host.exe  bin\oino.exe  bin\extensions\<pkg>\
# (scripts/release/pack-host.sh). The host writes its own rotated
# <data dir>\host.log on Windows.

$HostExe = Join-Path $BinDir "unipi-oino-host.exe"
$TaskName = "UniPi Oino host"
$Wrapper = Join-Path $InstallRoot "host-service.ps1"

function Stop-HostService {
    $task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    if ($task) { Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue }
    # Stopping the task ends the wrapper; the host it started is its own process.
    Get-Process -Name "unipi-oino-host" -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -eq $HostExe } |
        ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue; $_.WaitForExit(10000) | Out-Null }
}

function Install-HostTree([string]$Stage) {
    $From = Join-Path $Stage "bin"
    foreach ($name in @("unipi-oino-host.exe", "oino.exe")) {
        if (-not (Test-Path (Join-Path $From $name))) { throw "archive layout changed: no bin\$name in the zip" }
    }
    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    foreach ($name in @("unipi-oino-host.exe", "oino.exe")) {
        Copy-Item -Force (Join-Path $From $name) (Join-Path $BinDir $name)
    }
    $Ext = Join-Path $BinDir "extensions"
    if (Test-Path (Join-Path $From "extensions")) {
        # Swap the whole tree: a package the new release dropped goes away.
        if (Test-Path $Ext) { Remove-Item -Recurse -Force $Ext }
        Copy-Item -Recurse (Join-Path $From "extensions") $Ext
    }
    Say "Placed $HostExe, oino.exe and the extensions in $BinDir"
}

function Install-HostRelease {
    if ($env:OINO_FROM_SOURCE -eq "1") { return $false }
    $Tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("oino-host-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Force -Path $Tmp | Out-Null
    try {
        $Zip = Join-Path $Tmp "host.zip"
        if (-not (Fetch-Asset "unipi-oino-host-.*-$Target\.zip$" $Zip)) { return $false }
        Expand-Archive -Path $Zip -DestinationPath (Join-Path $Tmp "x") -Force
        Stop-HostService
        Install-HostTree (Join-Path $Tmp "x")
        return $true
    } finally {
        Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
    }
}

function Install-HostSource {
    Install-Source   # oino.exe + bin\extensions (prepares the checkout)
    $Src = if ((Test-Path "Cargo.toml") -and (Test-Path "crates\oino-core")) { (Get-Location).Path } else { $SrcDir }
    Run "cargo" @("build", "--manifest-path", (Join-Path $Src "apps\unipi\Cargo.toml"), "--release", "-p", "unipi-host")
    Stop-HostService
    Copy-Item -Force (Join-Path $Src "apps\unipi\target\release\unipi-host.exe") $HostExe
}

# The logon task runs this wrapper, hidden. Task Scheduler's own "restart on
# failure" only covers a task that fails to *launch* (a non-zero exit code
# counts as completed), so the wrapper restarts the host itself: any non-zero
# exit restarts it (1, 2, 4 … 30 s backoff, reset after a minute up); 0 (a
# clean stop) and 75 (another host already owns the data dir) end it.
function Write-HostWrapper {
    $exe = $HostExe.Replace("'", "''")
    $body = @"
# Written by install.ps1 -HostInstall: keeps unipi-oino-host running.
`$delay = 1
while (`$true) {
    `$started = Get-Date
    & '$exe' serve --mode service
    `$code = `$LASTEXITCODE
    if (`$code -eq 0 -or `$code -eq 75) { exit `$code }
    if (((Get-Date) - `$started).TotalSeconds -gt 60) { `$delay = 1 }
    Start-Sleep -Seconds `$delay
    `$delay = [Math]::Min(`$delay * 2, 30)
}
"@
    Set-Content -Path $Wrapper -Value $body -Encoding UTF8
}

function Install-HostService {
    if ($NoService) { Say "No service (-NoService). Start the host by hand: `"$HostExe`" serve"; return }
    Write-HostWrapper
    $user = "$env:USERDOMAIN\$env:USERNAME"
    $action = New-ScheduledTaskAction -Execute "powershell.exe" `
        -Argument "-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$Wrapper`""
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $user
    # RestartCount/Interval: launch failures; ExecutionTimeLimit 0: never
    # killed after the default 3 days; IgnoreNew: one host per user.
    $settings = New-ScheduledTaskSettingsSet -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) `
        -ExecutionTimeLimit ([TimeSpan]::Zero) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
        -StartWhenAvailable -MultipleInstances IgnoreNew
    $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
    Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Settings $settings `
        -Principal $principal -Description "UniPi Oino host (agent sessions for the Unipi apps); restarts when it fails" -Force | Out-Null
    Say "Registered the logon task `"$TaskName`" ($Wrapper)."
    if ($NoStart) { Say "Not started (-NoStart). Start it: Start-ScheduledTask -TaskName '$TaskName'"; return }
    Start-ScheduledTask -TaskName $TaskName
    Say "The host runs as the logon task `"$TaskName`" (it starts again at every logon)."
}

function Uninstall-Host {
    Stop-HostService
    if (Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue) {
        Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
    }
    Remove-Item -Force $Wrapper, $HostExe -ErrorAction SilentlyContinue
    Say "Removed the logon task and $HostExe."
    Say "Kept: oino.exe, $BinDir\extensions and the host data (~\.oino\host — pairings, key)."
}

function Install-Host {
    if ($DryRun) {
        Say "would install unipi-oino-host + oino ($Target) into $BinDir from $BaseUrl"
        if (-not $NoService) { Say "  service: logon task `"$TaskName`" running $Wrapper (restarts on failure)" }
        return
    }
    if (Install-HostRelease) {
        Say "Installed the host into $InstallRoot"
    } else {
        Say "No host release for $Target (or download failed); building from source."
        Install-HostSource
    }
    Add-BinDirToPath
    Install-HostService
    Say "Done. Pair the desktop or phone app: unipi-oino-host pair"
}

if ($App -and $HostInstall) { throw "-App and -HostInstall are separate installs; run them one at a time" }
if ($Uninstall -and -not $HostInstall) { throw "-Uninstall goes with -HostInstall" }
if ($HostInstall) {
    if ($Uninstall) { Uninstall-Host } else { Install-Host }
    exit 0
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
