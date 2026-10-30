# Back up Punchable's SurrealDB (punchable/main) with `surreal export`, gzipped.
#   powershell -ExecutionPolicy Bypass -File db\backup.ps1 [-Namespace punchable] [-Database main]
# Writes ~/punchable-data\db\punchable\backups\punchable-<ns>-<db>-<UTC time>.surql.gz (gitignored).
# Note: decisions.jsonl is the canonical history and `python db/load.py` rebuilds everything from the files;
# an export is a fast snapshot of the derived database, not a substitute for the files.
param([string]$Namespace = 'punchable', [string]$Database = 'main')
$ErrorActionPreference = 'Stop'
$exe = if ($env:PUNCH_SURREAL) { $env:PUNCH_SURREAL } else { Join-Path $env:USERPROFILE '.punchable\bin\surreal.exe' }
$cfg = @{}
foreach ($line in Get-Content (Join-Path $env:USERPROFILE '.punchable\secrets.env')) {
    if ($line -match '^\s*([A-Z_]+)\s*=\s*(.*)\s*$') { $cfg[$Matches[1]] = $Matches[2] }
}
$port = if ($cfg['SURREAL_PORT']) { $cfg['SURREAL_PORT'] } else { '8797' }
. (Join-Path $PSScriptRoot '..\tools\local_env.ps1')   # COMEDY_DIR etc. from ~/.punchable/punchable.env
if ($env:PUNCH_COMEDY) { $PkComedy = $env:PUNCH_COMEDY }
$dir = (Join-Path $PkComedy 'db\punchable\backups')
New-Item -ItemType Directory -Force $dir | Out-Null
$stamp = (Get-Date).ToUniversalTime().ToString('yyyyMMdd-HHmmss')
$out = Join-Path $dir "punchable-$Namespace-$Database-$stamp.surql"
$env:SURREAL_USER = $cfg['SURREAL_USER']; $env:SURREAL_PASS = $cfg['SURREAL_PASS']
try {
    & $exe export --endpoint "http://127.0.0.1:$port" --namespace $Namespace --database $Database $out
    if ($LASTEXITCODE -ne 0) { throw "surreal export failed ($LASTEXITCODE)" }
} finally { Remove-Item Env:SURREAL_PASS -ErrorAction SilentlyContinue }
$in = [IO.File]::OpenRead($out)
$gz = New-Object IO.Compression.GZipStream ([IO.File]::Create("$out.gz")), ([IO.Compression.CompressionLevel]::Optimal)
$in.CopyTo($gz); $gz.Dispose(); $in.Dispose()
Remove-Item $out -Confirm:$false
$size = [math]::Round((Get-Item "$out.gz").Length / 1MB, 2)
Write-Output "backup: $out.gz ($size MB)"
