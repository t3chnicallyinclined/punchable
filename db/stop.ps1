# Stop Punchable's local SurrealDB (the process started by db\start.ps1). Data stays on disk.
#   powershell -ExecutionPolicy Bypass -File db\stop.ps1
$ErrorActionPreference = 'Stop'
$root = (Join-Path $HOME 'punchable-data\db\punchable')
$pidFile = Join-Path $root 'surreal.pid'
$secrets = Join-Path $env:USERPROFILE '.punchable\secrets.env'
$port = 8797
if (Test-Path $secrets) {
    foreach ($line in Get-Content $secrets) { if ($line -match '^\s*SURREAL_PORT\s*=\s*(\d+)') { $port = [int]$Matches[1] } }
}
$ids = @()
if (Test-Path $pidFile) { $ids += [int](Get-Content $pidFile) }
$l = Get-NetTCPConnection -LocalAddress 127.0.0.1 -LocalPort $port -State Listen -ErrorAction SilentlyContinue
if ($l) { $ids += $l[0].OwningProcess }
$stopped = $false
foreach ($id in ($ids | Select-Object -Unique)) {
    $p = Get-Process -Id $id -ErrorAction SilentlyContinue
    if ($p -and $p.ProcessName -eq 'surreal') {
        Stop-Process -Id $id -Confirm:$false
        $p.WaitForExit(15000) | Out-Null
        Write-Output "stopped surreal pid $id"
        $stopped = $true
    }
}
if (Test-Path $pidFile) { Remove-Item $pidFile -Confirm:$false }
if (-not $stopped) { Write-Output "not running" }
