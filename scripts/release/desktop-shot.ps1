# Launch a desktop binary with --demo in the interactive console session (SSH sessions have
# no desktop) and take a full-screen screenshot from that session. Used by build-host.sh on
# Windows build hosts; needs the build user to be logged on at the console.
#
#   desktop-shot.ps1 -Exe C:\...\oino-desktop.exe -Png C:\...\desktop-demo.png [-Wait 20]
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(Mandatory = $true)][string]$Png,
    [int]$Wait = 20
)
# Continue: native stderr (schtasks "task not found") must not be terminating on PS 5.1.
$ErrorActionPreference = 'Continue'
$work = Join-Path $env:TEMP ("oino-shot-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path (Join-Path $work 'agent') -Force | Out-Null
$name = 'oino-release-shot'
$inner = Join-Path $work 'shot.ps1'
$log = Join-Path $work 'shot.log'
# Runs in the console session: start the app, wait, grab the screen, stop the app.
@"
`$ErrorActionPreference = 'Continue'
`$env:OINO_AGENT_DIR = '$work\agent'
`$env:OINO_REDUCE_MOTION = '1'
`$p = Start-Process -FilePath '$Exe' -ArgumentList '--demo' -PassThru
Start-Sleep -Seconds $Wait
`$alive = -not `$p.HasExited
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
`$b = [System.Windows.Forms.SystemInformation]::VirtualScreen
`$bmp = New-Object System.Drawing.Bitmap `$b.Width, `$b.Height
`$g = [System.Drawing.Graphics]::FromImage(`$bmp)
`$g.CopyFromScreen(`$b.Left, `$b.Top, 0, 0, `$bmp.Size)
`$bmp.Save('$Png', [System.Drawing.Imaging.ImageFormat]::Png)
if (`$alive) { Stop-Process -Id `$p.Id -Force -ErrorAction SilentlyContinue }
"alive=`$alive exit=`$(`$p.ExitCode)" | Set-Content '$log'
"@ | Set-Content -Path $inner -Encoding UTF8

if (Test-Path $Png) { Remove-Item $Png -Force }
$tr = "powershell.exe -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$inner`""
cmd /c "schtasks /delete /tn $name /f >nul 2>&1"
schtasks /create /tn $name /tr $tr /sc once /st 23:59 /it /ru $env:USERNAME /f | Out-Null
schtasks /run /tn $name | Out-Null
$deadline = (Get-Date).AddSeconds($Wait + 60)
while (-not (Test-Path $log) -and (Get-Date) -lt $deadline) { Start-Sleep -Seconds 2 }
cmd /c "schtasks /delete /tn $name /f >nul 2>&1"
if (-not (Test-Path $log)) { Write-Output 'no result from the console session (is the user logged on?)'; exit 1 }
$result = (Get-Content $log) -join ' '
Write-Output $result
Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
if ($result -notmatch 'alive=True') { exit 1 }
