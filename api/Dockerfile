# Punchable API (hosted on the server). Build context = repo root:
#   infra/ship.sh build api   ->   docker build -f api/Dockerfile .   (from `git archive HEAD` on the server)
# Slim Python, wheels only (no compilers anywhere), non-root uid 10001, read-only root filesystem: the only writes
# go to /tmp (an emptyDir in infra/k8s/30-api.yaml). No ollama, no claude: drafts and ask are queued for the home
# worker. The same image runs the dbload pod (python db/push_remote.py apply).
# Base pinned by digest (python:3.13-slim-bookworm, 2026-10-22).
FROM python:3.13-slim-bookworm@sha256:e32be0e55814a08977501ca863271d9bd02443710db8f2afaf8372e2c4508530 AS deps
COPY api/requirements.txt /tmp/requirements.txt
RUN python -m venv /venv \
 && /venv/bin/pip install --no-cache-dir --only-binary=:all: -r /tmp/requirements.txt \
 && /venv/bin/pip uninstall -y pip \
 && rm -rf /venv/lib/python3.13/site-packages/setuptools* /venv/lib/python3.13/site-packages/_distutils_hack

FROM python:3.13-slim-bookworm@sha256:e32be0e55814a08977501ca863271d9bd02443710db8f2afaf8372e2c4508530
RUN python -m pip uninstall -y pip setuptools wheel >/dev/null 2>&1 || true
COPY --from=deps /venv /venv
WORKDIR /app
COPY api/entry.py api/
COPY api-dev/server.py api-dev/auth.py api-dev/creative_api.py api-dev/surreal_kb.py api-dev/ai_overlay.py \
     api-dev/merge_api.py api-dev/ask_api.py api-dev/mysets_api.py api-dev/memos_api.py api-dev/ingest_api.py      api-dev/suggest_api.py api-dev/
COPY db/sdb.py db/kbmap.py db/load.py db/push_remote.py db/srcpaths.py db/schema.surql db/schema_extra.surql db/
# the legal rules gsync applies (court material is never stored); only this file of gsync, read by api-dev/ingest_api.py
COPY gsync/rules.py gsync/
COPY tools/explorer/kb.py tools/explorer/
# every creative module: a hand-kept list shipped an image without notebook.py/merge.py/write.py (boot ImportError)
COPY creative/*.py creative/
COPY infer/__init__.py infer/scriptdiff.py infer/
RUN /venv/bin/python -m compileall -q /app \
 && cd /app/api-dev && /venv/bin/python -c "import sys; sys.path[:0]=['.','..']; import server, merge_api, creative_api, ask_api, mysets_api, memos_api, ingest_api, suggest_api"
ENV PATH=/venv/bin:$PATH \
    HOME=/tmp \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    MALLOC_ARENA_MAX=2 \
    PK_AUTH=1 \
    PK_DATA=/tmp/punchable \
    PK_DBLOAD_DIR=/tmp/pk-dbload \
    COMEDY_DIR=/tmp/punchable/comedy \
    PORT=8080
USER 10001:10001
EXPOSE 8080
ENTRYPOINT ["python", "/app/api/entry.py"]
