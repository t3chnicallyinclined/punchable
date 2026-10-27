# Punchable web: static Expo web export (app/), served by unprivileged nginx on 8080. Build context = repo root
# (infra/ship.sh build web runs `docker build -f infra/web/Dockerfile .` on the server from `git archive HEAD`).
# Base images pinned by digest.
FROM node:22-alpine@sha256:fe4f6d74e20370acb04ad889077d14d0f64a9c8ea6a42bc60cddf1e844699914 AS build
WORKDIR /src/app
COPY app/package.json app/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY app/ ./
# Public, non-secret build-time config for the web bundle (EXPO_PUBLIC_* is inlined by Metro).
ARG PK_DOMAIN=punchable.io
ARG PK_AUTH_HOST=auth.punchable.io
ENV EXPO_PUBLIC_API_URL=https://${PK_DOMAIN}/api \
    EXPO_PUBLIC_OIDC_ISSUER=https://${PK_AUTH_HOST}/realms/punchable \
    EXPO_PUBLIC_OIDC_CLIENT_ID=pk-web \
    EXPO_PUBLIC_OIDC_REDIRECT=https://${PK_DOMAIN}/auth/callback \
    EXPO_NO_TELEMETRY=1 CI=1
RUN npx expo export --platform web --output-dir /out
# app/public/index.html names https://punchable.io in its og: tags; keep PK_DOMAIN the one place the host is set.
RUN sed -i "s#https://punchable.io#https://${PK_DOMAIN}#g" /out/index.html
# app/public/dev-fixtures holds real material for local dev only; it must never be served (2026-10-26 leak fix).
RUN rm -rf /out/dev-fixtures && ! find /out -path "*dev-fixtures*" | grep -q .

FROM nginxinc/nginx-unprivileged@sha256:0c79d56aee561a1d81c63f00eee5fb5fe29279560cdc55e91425133104c7fbe6
COPY infra/web/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /out /usr/share/nginx/html
USER 101
EXPOSE 8080
