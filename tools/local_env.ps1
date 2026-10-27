# Load your private local settings into this PowerShell session (dot-source it):
#   . .\tools\local_env.ps1
# Reads $env:PUNCHABLE_HOME\punchable.env (default %USERPROFILE%\.punchable\punchable.env): KEY=VALUE lines, # comments.
# Typical keys: COMEDY_DIR (your data folder), PK_CACHE_DIR, PK_WHISPER_PY, PK_LAUGHS_PY. No secrets belong in it
# (secrets.env holds those). Variables already set in the session win. Sets $PkEnv (hashtable of what the file holds)
# and $PkComedy (the data folder: COMEDY_DIR, else %USERPROFILE%\punchable-data).
$PkHome = if ($env:PUNCHABLE_HOME) { $env:PUNCHABLE_HOME } else { Join-Path $env:USERPROFILE '.punchable' }
$PkEnv = [ordered]@{}
$PkEnvFile = Join-Path $PkHome 'punchable.env'
if (Test-Path $PkEnvFile) {
    foreach ($line in Get-Content $PkEnvFile) {
        if ($line -match '^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*?)\s*$') {
            $k = $Matches[1]; $v = [Environment]::ExpandEnvironmentVariables($Matches[2].Trim('"'))
            $PkEnv[$k] = $v
            if (-not [Environment]::GetEnvironmentVariable($k)) { Set-Item -Path "Env:$k" -Value $v }
        }
    }
}
$PkComedy = if ($env:COMEDY_DIR) { $env:COMEDY_DIR } else { Join-Path $env:USERPROFILE 'punchable-data' }
# cmd.exe prefix that sets the same variables for a scheduled task: set "K=V"&& set "K2=V2"&&
$PkCmdSet = ($PkEnv.Keys | ForEach-Object { "set `"$_=$($PkEnv[$_])`"&& " }) -join ''
