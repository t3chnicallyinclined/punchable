# Punchable API (hosted on the server). Build context = repo root:
#   infra/ship.sh build api   ->   docker build -f api/Dockerfile .   (from `git archive HEAD` on the server)
# Slim Python, wheels only (no compilers anywhere), non-root uid 10001, read-only root filesystem: the only writes
# go to /tmp (an emptyDir in infra/k8s/30-api.yaml). No ollama, no claude: drafts and ask are queued for the hosted
# worker (infra/worker/Dockerfile). The same image runs the dbload pod (python db/push_remote.py apply).
# Base pinned by digest (python:3.13-slim-bookworm, 2026-10-22).
FROM python:3.13-slim-bookworm@sha256:e32be0e55814a08977501ca863271d9bd02443710db8f2afaf8372e2c4508530 AS deps
COPY api/requirements.txt /tmp/requirements.txt
RUN python -m venv /venv \
 && /venv/bin/pip install --no-cache-dir --only-binary=:all: -r /tmp/requirements.txt \
 && /venv/bin/pip uninstall -y pip \
 && rm -rf /venv/lib/python3.13/site-packages/setuptools* /venv/lib/python3.13/site-packages/_distutils_hack
# the bge-small-en-v1.5 ONNX export (MIT), checked against the hub's hashes and the fingerprint vector
# (tools/kb/fetch_bge.py), for query and passage vectors in the API (api-dev/search_api.py). Model files only.
# PK_WITH_BGE=0 skips it; the API then searches on words alone and says so at start.
ARG PK_WITH_BGE=1
COPY engine/*.py /src/engine/
COPY tools/kb/fetch_bge.py /src/tools/kb/fetch_bge.py
RUN if [ "$PK_WITH_BGE" = 1 ]; then /venv/bin/python /src/tools/kb/fetch_bge.py --dest /opt/punchable/models/bge-small-en-v1.5; \
    else mkdir -p /opt/punchable/models; fi

FROM python:3.13-slim-bookworm@sha256:e32be0e55814a08977501ca863271d9bd02443710db8f2afaf8372e2c4508530
RUN python -m pip uninstall -y pip setuptools wheel >/dev/null 2>&1 || true
COPY --from=deps /venv /venv
COPY --from=deps /opt/punchable/models /opt/punchable/models
WORKDIR /app
COPY api/entry.py api/
# every api-dev module: a hand-kept list shipped an image without say_api.py (health 500, pod never ready)
COPY api-dev/*.py api-dev/
# the Prometheus registry (api-dev/metrics.py) and the ONNX embedder (api-dev/search_api.py); the rest of engine/
# stays out of this image on purpose
COPY engine/__init__.py engine/prom.py engine/embed.py engine/
COPY db/sdb.py db/kbmap.py db/load.py db/push_remote.py db/srcpaths.py db/schema.surql db/schema_extra.surql db/
# the legal rules gsync applies (court material is never stored); only this file of gsync, read by api-dev/ingest_api.py
COPY gsync/rules.py gsync/
COPY tools/explorer/kb.py tools/explorer/
# every creative module: a hand-kept list shipped an image without notebook.py/merge.py/write.py (boot ImportError)
COPY creative/*.py creative/
COPY infer/__init__.py infer/scriptdiff.py infer/
RUN /venv/bin/python -m compileall -q /app \
 && cd /app/api-dev && /venv/bin/python -c "import sys; sys.path[:0]=['.','..']; import importlib, pathlib, server; [importlib.import_module(p.stem) for p in pathlib.Path('.').glob('*_api.py')]" \
 && cd /app && PK_BGE_DIR=/opt/punchable/models/bge-small-en-v1.5 /venv/bin/python -c "import sys; sys.path[:0]=['api-dev']; import search_api as S; e = S.make_embedder(); assert e is None or len(e.embed(['x'])[0]) == 384"
# the git sha of the shipped tree (infra/deploy.sh build passes it): pk_build_info and the crash-report release
ARG PK_BUILD_SHA=dev
ENV PK_BUILD_SHA=$PK_BUILD_SHA \
    PATH=/venv/bin:$PATH \
    HOME=/tmp \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    MALLOC_ARENA_MAX=2 \
    PK_AUTH=1 \
    PK_DATA=/tmp/punchable \
    PK_BGE_DIR=/opt/punchable/models/bge-small-en-v1.5 \
    PK_DBLOAD_DIR=/tmp/pk-dbload \
    COMEDY_DIR=/tmp/punchable/comedy \
    PORT=8080
USER 10001:10001
EXPOSE 8080
ENTRYPOINT ["python", "/app/api/entry.py"]
