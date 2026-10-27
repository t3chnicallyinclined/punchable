# Register (or remove) gsync as a per-user scheduled task that starts at logon and loops (python -m gsync run --watch).
#   powershell -ExecutionPolicy Bypass -File gsync\install_task.ps1 [-Remove] [-Interval 60]
# Runs with the gsync venv (C:\Users\<you>\.venvs\gsync). No secret is passed here: tokens are read from
# %USERPROFILE%\.punchable by gsync itself; settings (COMEDY_DIR, ...) from ~/.punchable/punchable.env
# (tools/local_env.ps1) are set on the task's command line. Output: <data folder>\db\punchable\logs\gsync.log
param([switch]$Remove, [int]$Interval = 60)
$ErrorActionPreference = "Stop"
$Name = "Punchable gsync"
if ($Remove) { Unregister-ScheduledTask -TaskName $Name -Confirm:$false; "removed $Name"; return }
$Desk = Split-Path -Parent $PSScriptRoot
$Py = Join-Path $env:USERPROFILE ".venvs\gsync\Scripts\python.exe"
if (-not (Test-Path $Py)) { throw "gsync venv missing: $Py (gsync/SETUP.md step 10)" }
. (Join-Path $PSScriptRoot "..\tools\local_env.ps1")   # COMEDY_DIR etc. from ~/.punchable/punchable.env
$Log = Join-Path $PkComedy "db\punchable\logs"
New-Item -ItemType Directory -Force $Log | Out-Null
$Action = New-ScheduledTaskAction -Execute "cmd.exe" -WorkingDirectory $Desk `
  -Argument "/c `"${PkCmdSet}set PYTHONUTF8=1&& `"$Py`" -m gsync run --watch --interval $Interval >> `"$Log\gsync.log`" 2>&1`""
$Trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
$Settings = New-ScheduledTaskSettingsSet -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) `
  -ExecutionTimeLimit (New-TimeSpan -Days 0) -StartWhenAvailable -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
Register-ScheduledTask -TaskName $Name -Action $Action -Trigger $Trigger -Settings $Settings `
  -Description "Punchable: Google Docs <-> Punchable sync every $Interval s (gsync/GSYNC.md)" -Force | Out-Null
Start-ScheduledTask -TaskName $Name
"registered and started: $Name (log: $Log\gsync.log)"
