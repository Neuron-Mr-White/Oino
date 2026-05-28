# Oino Windows/PowerShell installer.
# Environment variables:
#   OINO_REPO      git URL to clone when not in a checkout
#   OINO_REF       git ref to checkout after clone
#   OINO_PREFIX    install prefix; default: $HOME\.local
#   OINO_DIR       source checkout dir; default: $HOME\.cache\oino\source
#   OINO_DRY_RUN   set to 1 to print actions without running build/install

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Say([string]$Message) {
    Write-Host $Message
}

function Have([string]$Name) {
    return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

function EnvValue([string]$Name) {
    return [System.Environment]::GetEnvironmentVariable($Name)
}

$DryRun = (EnvValue "OINO_DRY_RUN") -eq "1"

function ConvertTo-PlainHashtable($Value) {
    if ($null -eq $Value) { return $null }
    if ($Value -is [System.Collections.IDictionary]) {
        $Hash = @{}
        foreach ($Key in $Value.Keys) { $Hash[$Key] = ConvertTo-PlainHashtable $Value[$Key] }
        return $Hash
    }
    if ($Value -is [pscustomobject]) {
        $Hash = @{}
        foreach ($Property in $Value.PSObject.Properties) {
            $Hash[$Property.Name] = ConvertTo-PlainHashtable $Property.Value
        }
        return $Hash
    }
    if (($Value -is [System.Collections.IEnumerable]) -and ($Value -isnot [string])) {
        return @($Value | ForEach-Object { ConvertTo-PlainHashtable $_ })
    }
    return $Value
}

function Run([Parameter(ValueFromRemainingArguments = $true)][string[]]$Command) {
    if ($Command.Count -eq 0) { return }
    Say "+ $($Command -join ' ')"
    if (-not $DryRun) {
        $exe = $Command[0]
        $args = @()
        if ($Command.Count -gt 1) {
            $args = $Command[1..($Command.Count - 1)]
        }
        $commandInfo = Get-Command $exe -ErrorAction Stop
        & $exe @args
        if (-not $?) {
            throw "command failed: $($Command -join ' ')"
        }
        if (($commandInfo.CommandType -eq "Application" -or $commandInfo.CommandType -eq "ExternalScript") -and $LASTEXITCODE -ne 0) {
            throw "command failed with exit code ${LASTEXITCODE}: $($Command -join ' ')"
        }
    }
}

function EnsureDir([string]$Path) {
    Say "+ mkdir -Force $Path"
    if (-not $DryRun) {
        New-Item -ItemType Directory -Force -Path $Path | Out-Null
    }
}

function RemovePath([string]$Path) {
    Say "+ remove-item -recurse -force $Path"
    if (-not $DryRun) {
        Remove-Item -Recurse -Force $Path
    }
}

function CopyPath([string]$Source, [string]$Destination) {
    Say "+ copy-item -recurse -force $Source $Destination"
    if (-not $DryRun) {
        Copy-Item -Recurse -Force $Source $Destination
    }
}

$RepoEnv = EnvValue "OINO_REPO"
$PrefixEnv = EnvValue "OINO_PREFIX"
$DirEnv = EnvValue "OINO_DIR"
$Repo = if ($RepoEnv) { $RepoEnv } else { "https://github.com/Neuron-Mr-White/Oino.git" }
$Prefix = if ($PrefixEnv) { $PrefixEnv } else { Join-Path $HOME ".local" }
$BinDir = Join-Path $Prefix "bin"
$SrcDir = if ($DirEnv) { $DirEnv } else { Join-Path $HOME ".cache\oino\source" }

if ((Test-Path "Cargo.toml") -and (Select-String -Path "Cargo.toml" -Pattern "oino-app" -Quiet)) {
    $Src = (Get-Location).Path
} else {
    $Src = $SrcDir
    if (-not (Test-Path (Join-Path $Src ".git"))) {
        if (-not (Have "git")) {
            throw "git is required to clone Oino. Install git, or run this script from an Oino source checkout."
        }
        $Parent = Split-Path -Parent $Src
        if ($Parent) { EnsureDir $Parent }
        Run git clone $Repo $Src
    }
    $RefEnv = EnvValue "OINO_REF"
    if ($RefEnv) {
        Run git -C $Src fetch --all --tags
        Run git -C $Src checkout $RefEnv
    }
}

if (-not (Have "cargo")) {
    if (Have "rustup") {
        Say "rustup exists; using it"
    } else {
        if (Have "winget") {
            Say "Installing Rust with winget..."
            Run winget install --id Rustlang.Rustup -e --accept-package-agreements --accept-source-agreements
        } elseif (Have "curl") {
            Say "Installing Rust with rustup-init..."
            $TempDir = if (EnvValue "TEMP") { EnvValue "TEMP" } else { [System.IO.Path]::GetTempPath() }
            $RustupInit = Join-Path $TempDir "rustup-init.exe"
            Run curl.exe -fsSL https://win.rustup.rs/x86_64 -o $RustupInit
            Run $RustupInit -y --profile minimal
        } else {
            throw "cargo is missing and neither winget nor curl is available. Install Rust from https://rustup.rs/ and rerun."
        }
    }
    $CargoEnv = Join-Path $HOME ".cargo\env.ps1"
    if ((-not $DryRun) -and (Test-Path $CargoEnv)) { . $CargoEnv }
}

Run cargo build --manifest-path (Join-Path $Src "Cargo.toml") -p oino-app --bin oino --release
EnsureDir $BinDir

$BuiltExe = Join-Path $Src "target\release\oino.exe"
$InstalledExe = Join-Path $BinDir "oino.exe"
CopyPath $BuiltExe $InstalledExe

$PackagesDir = Join-Path $Src "extensions\built-in"
if (Test-Path $PackagesDir) {
    $OinoHomeEnv = EnvValue "OINO_HOME"
    $OinoHome = if ($OinoHomeEnv) { $OinoHomeEnv } else { $HOME }
    $TargetDir = Join-Path $OinoHome ".oino\extension-packages"
    $SettingsPath = Join-Path $OinoHome ".oino\settings.json"
    $PackageIds = @(
        "oino.9router",
        "oino.footer_status",
        "oino.ralph_loop",
        "oino.mode_sandbox",
        "oino.notify",
        "oino.craft_skill",
        "oino.vcc",
        "oino.ask_user"
    )
    EnsureDir $TargetDir
    foreach ($PackageId in $PackageIds) {
        $PackageFile = Get-ChildItem -Path $PackagesDir -Filter "oino.package.json" -Recurse |
            Where-Object { ((Get-Content $_.FullName -Raw | ConvertFrom-Json).id -eq $PackageId) } |
            Select-Object -First 1
        if (-not $PackageFile) { throw "missing built-in package $PackageId" }
        $Destination = Join-Path $TargetDir $PackageId
        if (Test-Path $Destination) { RemovePath $Destination }
        CopyPath $PackageFile.DirectoryName $Destination
        $SourceRecord = @{ source = "builtin:$($PackageFile.Directory.Name)" } | ConvertTo-Json
        if (-not $DryRun) { $SourceRecord | Out-File -Encoding utf8 (Join-Path $Destination ".oino-install-source.json") }
    }
    if ((-not $DryRun) -and (Test-Path $SettingsPath)) {
        $Settings = ConvertTo-PlainHashtable (Get-Content $SettingsPath -Raw | ConvertFrom-Json)
    } else {
        $Settings = @{}
    }
    if (-not $Settings.ContainsKey("extensions")) { $Settings["extensions"] = @{} }
    if (-not $Settings["extensions"].ContainsKey("packages")) { $Settings["extensions"]["packages"] = @{} }
    foreach ($PackageId in $PackageIds) { $Settings["extensions"]["packages"][$PackageId] = "enabled" }
    if (-not $DryRun) {
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $SettingsPath) | Out-Null
        $Settings | ConvertTo-Json -Depth 20 | Out-File -Encoding utf8 $SettingsPath
    }
}

if (($env:PATH -split [IO.Path]::PathSeparator) -notcontains $BinDir) {
    Say "Add this to PATH if needed: $BinDir"
}
Say "Oino installed: $InstalledExe"
Say "Start with: oino"
