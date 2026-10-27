# Start Punchable's local SurrealDB 3.3.4 (SurrealKV on disk, loopback only).
#   powershell -ExecutionPolicy Bypass -File db\start.ps1
# Data:   ~/punchable-data\db\punchable\data   (gitignored by comedy\db\.gitignore)
# Logs:   ~/punchable-data\db\punchable\logs\surreal.log
# Secrets: %USERPROFILE%\.punchable\secrets.env (SURREAL_USER, SURREAL_PASS, SURREAL_PORT); passed by environment,
#          never on the command line.
$ErrorActionPreference = 'Stop'
$secrets = Join-Path $env:USERPROFILE '.punchable\secrets.env'
$exe = if ($env:PUNCH_SURREAL) { $env:PUNCH_SURREAL } else { Join-Path $env:USERPROFILE '.punchable\bin\surreal.exe' }
. (Join-Path $PSScriptRoot '..\tools\local_env.ps1')   # COMEDY_DIR etc. from ~/.punchable/punchable.env
if ($env:PUNCH_COMEDY) { $PkComedy = $env:PUNCH_COMEDY }
$root = (Join-Path $PkComedy 'db\punchable')
$data = Join-Path $root 'data'
$logs = Join-Path $root 'logs'
$pidFile = Join-Path $root 'surreal.pid'

if (-not (Test-Path $secrets)) { throw "missing $secrets (SURREAL_USER / SURREAL_PASS / SURREAL_PORT)" }
if (-not (Test-Path $exe)) { throw "missing SurrealDB binary $exe (official 3.3.4 Windows build)" }
$cfg = @{}
foreach ($line in Get-Content $secrets) {
    if ($line -match '^\s*([A-Z_]+)\s*=\s*(.*)\s*$') { $cfg[$Matches[1]] = $Matches[2] }
}
$port = if ($cfg['SURREAL_PORT']) { [int]$cfg['SURREAL_PORT'] } else { 8797 }

$ver = (& $exe version) -join ' '
if ($ver -notmatch '^3\.(3\.([1-9]|\d\d)|([4-9]|\d\d)\.)') { throw "SurrealDB $ver is too old: need >= 3.3.1 (the vector index lives in RAM and is rebuilt on start)" }

$busy = Get-NetTCPConnection -LocalAddress 127.0.0.1 -LocalPort $port -State Listen -ErrorAction SilentlyContinue
if ($busy) {
    $p = Get-Process -Id $busy[0].OwningProcess -ErrorAction SilentlyContinue
    if ($p -and $p.ProcessName -eq 'surreal') { Write-Output "already running: pid $($p.Id) on 127.0.0.1:$port"; exit 0 }
    throw "port $port is in use by $($p.ProcessName) (pid $($busy[0].OwningProcess))"
}

New-Item -ItemType Directory -Force $data, $logs | Out-Null
$env:SURREAL_USER = $cfg['SURREAL_USER']
$env:SURREAL_PASS = $cfg['SURREAL_PASS']
$env:SURREAL_NO_BANNER = 'true'
$store = 'surrealkv://' + ($data -replace '\\', '/')
$proc = Start-Process -FilePath $exe -ArgumentList @('start', '--log', 'info', '--bind', "127.0.0.1:$port", '--deny-scripting', $store) `
    -RedirectStandardOutput (Join-Path $logs 'surreal.log') -RedirectStandardError (Join-Path $logs 'surreal.err.log') `
    -WindowStyle Hidden -PassThru
Remove-Item Env:SURREAL_PASS
Set-Content -Path $pidFile -Value $proc.Id -Encoding ascii

for ($i = 0; $i -lt 100; $i++) {
    Start-Sleep -Milliseconds 200
    if ($proc.HasExited) { throw "surreal exited ($($proc.ExitCode)); see $logs" }
    if (Get-NetTCPConnection -LocalAddress 127.0.0.1 -LocalPort $port -State Listen -ErrorAction SilentlyContinue) {
        Write-Output "SurrealDB $ver running: pid $($proc.Id), ws://127.0.0.1:$port/rpc, store $store"
        exit 0
    }
}
throw "surreal did not start listening on $port; see $logs"
