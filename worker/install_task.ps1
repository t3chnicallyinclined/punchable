# DEVELOPMENT ONLY (since 2026-10-27 the worker runs on the server: infra/k8s/35-worker.yaml, worker/README.md).
# Register (or remove) the PC worker as a per-user scheduled task that starts at logon.
#   powershell -ExecutionPolicy Bypass -File worker\install_task.ps1 [-Remove]
param([switch]$Remove)
$ErrorActionPreference = "Stop"
$Name = "Punchable worker"
if ($Remove) { Unregister-ScheduledTask -TaskName $Name -Confirm:$false; "removed $Name"; return }
$Desk = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "..\tools\local_env.ps1")   # COMEDY_DIR etc. from ~/.punchable/punchable.env
$Py = if ($env:PK_WORKER_PY) { $env:PK_WORKER_PY } else { (Get-Command python).Source }
$Log = Join-Path $PkComedy "db\punchable\logs"
New-Item -ItemType Directory -Force $Log | Out-Null
# cmd /c so stdout+stderr go to the log; the secret is read from secrets.env by the worker, never passed here.
$Action = New-ScheduledTaskAction -Execute "cmd.exe" -WorkingDirectory $Desk `
  -Argument "/c `"${PkCmdSet}`"$Py`" -m worker >> `"$Log\worker.log`" 2>&1`""
$Trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
$Settings = New-ScheduledTaskSettingsSet -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) `
  -ExecutionTimeLimit (New-TimeSpan -Days 0) -StartWhenAvailable -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
Register-ScheduledTask -TaskName $Name -Action $Action -Trigger $Trigger -Settings $Settings `
  -Description "Punchable: runs queued drafts/ask/set jobs from punchable.io locally; pulls phone decisions" -Force | Out-Null
Start-ScheduledTask -TaskName $Name
"registered and started: $Name (log: $Log\worker.log)"
