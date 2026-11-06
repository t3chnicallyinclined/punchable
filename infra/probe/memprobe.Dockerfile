# T15: the pk-embed memory probe in the distroless base the Rust services will ship on (docs/RUST.md "Build, CI,
# images"; infra/README.md "Memory"). Build it OFF the server (docs/ARCHITECTURE.md §6: no cargo build on the node):
#   git archive HEAD | docker build -f infra/probe/memprobe.Dockerfile -t punchable-memprobe:<sha> -
#   (the committed tree only, as infra/ship.sh ships it; memprobe.Dockerfile.dockerignore also keeps rust/target out
#   if you build from a checkout with `.` as the context). The rust and distroless bases are not pinned by digest
#   yet: pin them when the first Rust service image is (T8).
# It prints, after the model loads, after one query, after 1,000 and after 1,000 passages, the `Anonymous:` line of
# /proc/self/smaps_rollup (the cgroup `anon` unit) and the resident set. Nothing is read or written but the model.
FROM rust:1.99.1-bookworm AS build
WORKDIR /src
COPY rust/ rust/
COPY db/schema.surql db/schema_extra.surql db/
COPY engine/config.hosted.toml engine/
RUN cd rust && cargo build --release --locked -p pk-embed --example memprobe \
 && cp target/release/examples/memprobe /memprobe

# the bge-small-en-v1.5 ONNX export, checked against the hub's hashes and the fingerprint (as api/Dockerfile does)
FROM python:3.13-slim-bookworm@sha256:e32be0e55814a08977501ca863271d9bd02443710db8f2afaf8372e2c4508530 AS model
COPY api/requirements.txt /tmp/requirements.txt
RUN python -m venv /venv && /venv/bin/pip install --no-cache-dir --only-binary=:all: -r /tmp/requirements.txt
COPY engine/*.py /src/engine/
COPY tools/kb/fetch_bge.py /src/tools/kb/fetch_bge.py
RUN /venv/bin/python /src/tools/kb/fetch_bge.py --dest /opt/punchable/models/bge-small-en-v1.5

FROM gcr.io/distroless/cc-debian12
COPY --from=build /memprobe /memprobe
COPY --from=model /opt/punchable/models /opt/punchable/models
USER 10001:10001
ENTRYPOINT ["/memprobe", "--dir", "/opt/punchable/models/bge-small-en-v1.5", "--threads", "1", "--n", "1000"]
