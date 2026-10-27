# Install the commit guard in this clone: pre-commit and pre-push hooks from tools/guard/hooks (core.hooksPath).
#   powershell -ExecutionPolicy Bypass -File tools\guard\install.ps1 [-Python C:\path\to\python.exe] [-Remove]
# The hooks call tools/guard/check.py with the Python recorded here (git config punchable.guardPython).
# Your private term list goes in %USERPROFILE%\.punchable\guard_terms.json and your data folder is COMEDY_DIR
# (or ~/.punchable/punchable.env); the index of your material is cached in %USERPROFILE%\.punchable\guard_cache.
param([string]$Python = "", [switch]$Remove)
$ErrorActionPreference = "Stop"
$Repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if ($Remove) {
  git -C $Repo config --unset core.hooksPath; git -C $Repo config --unset punchable.guardPython
  "guard removed"; return
}
if (-not $Python) {
  . (Join-Path $Repo "tools\local_env.ps1")
  $Python = if ($env:PK_GUARD_PY) { $env:PK_GUARD_PY } else { (Get-Command python -ErrorAction Stop).Source }
}
if (-not (Test-Path $Python)) { throw "python not found: $Python" }
git -C $Repo config core.hooksPath tools/guard/hooks
git -C $Repo config punchable.guardPython ($Python.Replace([char]92, [char]47))
& $Python (Join-Path $Repo "tools\guard\check.py") --rebuild
if ($LASTEXITCODE) { throw "guard index build failed" }
"guard installed: pre-commit + pre-push (python: $Python)"
