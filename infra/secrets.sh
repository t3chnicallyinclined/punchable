#!/usr/bin/env bash
# Create Punchable's k8s Secrets BY HAND on the server (never in git, never printed). Run ON the server (infra/ship.sh base).
# Idempotent: an existing Secret is never overwritten or rotated by this script.
#   punchable-surrealdb  SURREAL_USER=root, SURREAL_PASS=<64 hex, generated here>
# Values go through a process-substitution env file (printf is a shell builtin), so they never appear in argv/ps.
# `kubectl create` (not client-side apply) adds no last-applied-configuration annotation.
# The root password needs no off-box copy: `surreal export` carries data and DB users, and a restore runs under a
# fresh root (see the restore drill).
set -euo pipefail
K="microk8s kubectl"; NS=punchable
$K get ns $NS >/dev/null
if $K -n $NS get secret punchable-surrealdb >/dev/null 2>&1; then
  echo "punchable-surrealdb exists (left as is)"
else
  P="$(openssl rand -hex 32)"
  $K -n $NS create secret generic punchable-surrealdb --from-env-file=<(printf 'SURREAL_USER=root\nSURREAL_PASS=%s\n' "$P")
  unset P
  echo "created punchable-surrealdb"
fi
