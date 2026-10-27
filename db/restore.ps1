# Restore Punchable's SurrealDB from a db\backup.ps1 file (.surql or .surql.gz).
#   powershell -ExecutionPolicy Bypass -File db\restore.ps1 -File <backup> [-Namespace punchable] [-Database main]
# The target database is REMOVED first and re-created from the backup. Stop the API before restoring to main,
# and start it again afterwards (it checks the change feeds on start).
# If decisions.jsonl has lines the backup doesn't, the API's file watcher appends them (through the event) on
# start; `python db/load.py` is always the full rebuild from the files.
param([Parameter(Mandatory = $true)][string]$File, [string]$Namespace = 'punchable', [string]$Database = 'main')
$ErrorActionPreference = 'Stop'
$exe = if ($env:PUNCH_SURREAL) { $env:PUNCH_SURREAL } else { Join-Path $env:USERPROFILE '.punchable\bin\surreal.exe' }
$cfg = @{}
foreach ($line in Get-Content (Join-Path $env:USERPROFILE '.punchable\secrets.env')) {
    if ($line -match '^\s*([A-Z_]+)\s*=\s*(.*)\s*$') { $cfg[$Matches[1]] = $Matches[2] }
}
$port = if ($cfg['SURREAL_PORT']) { $cfg['SURREAL_PORT'] } else { '8797' }
$src = (Resolve-Path $File).Path
$tmp = $null
if ($src.EndsWith('.gz')) {
    $tmp = Join-Path $env:TEMP ("punchable-restore-" + [guid]::NewGuid().ToString('N') + ".surql")
    $gz = New-Object IO.Compression.GZipStream ([IO.File]::OpenRead($src)), ([IO.Compression.CompressionMode]::Decompress)
    $o = [IO.File]::Create($tmp); $gz.CopyTo($o); $o.Dispose(); $gz.Dispose()
    $src = $tmp
}
$env:SURREAL_USER = $cfg['SURREAL_USER']; $env:SURREAL_PASS = $cfg['SURREAL_PASS']
try {
    "DEFINE NAMESPACE IF NOT EXISTS $Namespace; USE NS $Namespace; REMOVE DATABASE IF EXISTS $Database; DEFINE DATABASE $Database STRICT;" |
        & $exe sql --endpoint "http://127.0.0.1:$port" --hide-welcome | Out-Null
    & $exe import --endpoint "http://127.0.0.1:$port" --namespace $Namespace --database $Database $src
    if ($LASTEXITCODE -ne 0) { throw "surreal import failed ($LASTEXITCODE)" }
} finally {
    Remove-Item Env:SURREAL_PASS -ErrorAction SilentlyContinue
    if ($tmp) { Remove-Item $tmp -Confirm:$false -ErrorAction SilentlyContinue }
}
Write-Output "restored $Namespace/$Database from $File"
