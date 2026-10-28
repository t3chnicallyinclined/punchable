# Punchable hosted worker (the server's writer). Build context = repo root:
#   infra/ship.sh build worker   ->   docker build -f infra/worker/Dockerfile .   (from `git archive HEAD` on the server)
# A separate image from api/Dockerfile on purpose: the API image deliberately holds no engine, no model and no
# worker code (it serves and queues); this one holds the engine (engine/, creative/, worker/), the Anthropic SDK,
# the bge-small ONNX model for the embedding kinds, and the API's crash.py + scrub.py (crash reports to Bugsink). No whisper, no laugh detector, no yt-dlp, no ffmpeg: the kinds
# that need audio are never claimed here (worker/__main__.py TRANSCRIBER_KINDS).
# Slim Python, wheels only (no compilers), non-root uid 10001, read-only root filesystem: the only writes go to /tmp
# (an emptyDir in infra/k8s/35-worker.yaml: the disposable workspace, the heartbeat, the day's spend ledger).
# Base pinned by digest (python:3.13-slim-bookworm, 2026-10-22; = api/Dockerfile).
FROM python:3.13-slim-bookworm@sha256:e32be0e55814a08977501ca863271d9bd02443710db8f2afaf8372e2c4508530 AS deps
COPY infra/worker/requirements.txt /tmp/requirements.txt
RUN python -m venv /venv \
 && /venv/bin/pip install --no-cache-dir --only-binary=:all: -r /tmp/requirements.txt \
 && /venv/bin/pip uninstall -y pip \
 && rm -rf /venv/lib/python3.13/site-packages/setuptools* /venv/lib/python3.13/site-packages/_distutils_hack
# the bge-small-en-v1.5 ONNX export (MIT), checked against the hub's hashes and the fingerprint vector
# (tools/kb/fetch_bge.py). Model files only: nothing of the comic's leaves or enters here. PK_WITH_BGE=0 skips it;
# the worker then does not claim embed/memolink/ask and says so at start.
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
COPY worker/*.py worker/
COPY engine/*.py engine/*.toml engine/
COPY creative/*.py creative/
COPY db/sdb.py db/kbmap.py db/srcpaths.py db/embedder.py db/
COPY gsync/rules.py gsync/memos.py gsync/__init__.py gsync/
COPY tools/explorer/kb.py tools/explorer/
COPY infer/*.py infer/
# crash reports: the API's SDK wrapper and scrubber, used by worker/crashreport.py (release punchable-worker@...)
COPY api-dev/crash.py api-dev/scrub.py api-dev/
RUN /venv/bin/python -m compileall -q /app \
 && cd /app && ENGINE_CONFIG=/app/engine/config.hosted.toml /venv/bin/python -c "import worker.__main__, worker.online, worker.execute, worker.crashreport, sentry_sdk, engine.providers, engine.budget, creative.askchat, creative.write, creative.memolink; from engine.config import Config; c = Config(); assert set(c.providers) == {'anthropic'}, c.providers"
ARG PK_BUILD_SHA=dev
ENV PK_BUILD_SHA=$PK_BUILD_SHA \
    PATH=/venv/bin:$PATH \
    HOME=/tmp \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    MALLOC_ARENA_MAX=2 \
    ENGINE_CONFIG=/app/engine/config.hosted.toml \
    PK_HOSTED=1 \
    PK_ONLINE_FIRST=1 \
    PK_WORKSPACE=/tmp/pk-worker \
    PK_CACHE_DIR=/tmp/pk-worker/cache \
    PK_BGE_DIR=/opt/punchable/models/bge-small-en-v1.5 \
    PK_HEARTBEAT_FILE=/tmp/pk-worker/heartbeat \
    PK_AI_SPEND_FILE=/tmp/pk-worker/ai_spend.json \
    PUNCH_SECRETS=/nonexistent
USER 10001:10001
# GET /metrics for Prometheus (worker/metrics.py; PK_METRICS_PORT): the only thing this image listens on
EXPOSE 9100
ENTRYPOINT ["python", "-m", "worker", "--hosted"]
