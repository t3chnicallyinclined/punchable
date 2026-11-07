# Register (or remove) pk-gsync, the Rust gsync, as a per-user scheduled task that starts at logon and loops
# (pk-gsync run --watch). The same task as gsync\install_task.ps1, with the same settings, but a DIFFERENT NAME, so the
# Python task can stay registered (disabled) as the way back:
#   powershell -ExecutionPolicy Bypass -File rust\crates\pk-gsync\install_task.ps1 [-Remove] [-Interval 60] [-Binary <path>]
#   powershell -ExecutionPolicy Bypass -File rust\crates\pk-gsync\install_task.ps1 -WhatIf     # print the task, register nothing
# Only ONE of the two tasks may be enabled: both write the same state files and rotate the same Keycloak token.
# No secret is passed here: tokens are read from %USERPROFILE%\.punchable by pk-gsync itself; settings (COMEDY_DIR,
# PK_ONLINE_FIRST, ...) from ~/.punchable/punchable.env (tools/local_env.ps1) are set on the task's command line.
# Output: <data folder>\db\punchable\logs\gsync.log (the same log file as the Python task).
param([switch]$Remove, [switch]$WhatIf, [int]$Interval = 60, [string]$Binary = "")
$ErrorActionPreference = "Stop"
$Name = "Punchable gsync (Rust)"
if ($Remove) { Unregister-ScheduledTask -TaskName $Name -Confirm:$false; "removed $Name"; return }
$Desk = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path   # the repo root: the working folder (the memo processor, Python, runs from here)
if (-not $Binary) { $Binary = Join-Path $env:USERPROFILE ".punchable\bin\pk-gsync.exe" }
if (-not (Test-Path $Binary)) { throw "pk-gsync missing: $Binary (cd rust; cargo build --release -p pk-gsync; copy target\release\pk-gsync.exe there, or pass -Binary)" }
. (Join-Path $Desk "tools\local_env.ps1")   # COMEDY_DIR etc. from ~/.punchable/punchable.env
$Log = Join-Path $PkComedy "db\punchable\logs"
$Argument = "/c `"${PkCmdSet}`"$Binary`" run --watch --interval $Interval >> `"$Log\gsync.log`" 2>&1`""
if ($WhatIf) { "would register '$Name' in $Desk"; "cmd.exe $Argument"; return }
New-Item -ItemType Directory -Force $Log | Out-Null
$Action = New-ScheduledTaskAction -Execute "cmd.exe" -WorkingDirectory $Desk -Argument $Argument
$Trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
$Settings = New-ScheduledTaskSettingsSet -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) `
  -ExecutionTimeLimit (New-TimeSpan -Days 0) -StartWhenAvailable -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
Register-ScheduledTask -TaskName $Name -Action $Action -Trigger $Trigger -Settings $Settings `
  -Description "Punchable: Google Docs <-> Punchable sync every $Interval s (pk-gsync, the Rust gsync)" -Force | Out-Null
Start-ScheduledTask -TaskName $Name
"registered and started: $Name (log: $Log\gsync.log)"
