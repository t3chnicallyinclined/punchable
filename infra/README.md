# Hosting Punchable (MicroK8s)

How the hosted version runs: one server with MicroK8s, a Keycloak you already run (in its own namespace), Cloudflare
DNS, and images built on the server into the MicroK8s registry (`localhost:32000`). Nothing here is needed to run
Punchable on your own computer; see the top-level README for that.

## Setup
1. `cp infra/config.env.example ~/.punchable/infra.env` and fill it in (domain, auth host, namespaces, server IP, ssh
   target). `infra/config.env` (gitignored) also works; `PK_INFRA_ENV=<file>` picks another one.
2. `infra/ship.sh <step>` sends the committed tree and that config file to the server over ssh and runs
   `infra/deploy.sh <step>` there.

| Step | What it does |
|---|---|
| `test` | imports the realm into a throwaway Keycloak (plain Docker) and checks it, then checks it fails on a broken realm |
| `base` | namespace, Secrets (generated on the server, never in git), NetworkPolicies, SurrealDB, db-init, a network check |
| `google-idp` | "Sign in with Google": the OAuth client id/secret from `~/.punchable/google-oauth.env`, over ssh stdin, into a Secret |
| `keycloak` | the realm ConfigMap and its own import job (other realms are not touched) |
| `keycloak-ingress` | the `auth.<domain>` Ingress and certificate |
| `build api\|web` / `rollout api\|web` | build the images on the server, then roll out pinned to the digest |
| `dbload [--check]` | pack your text/JSON data here (`db/push_remote.py`), load it into SurrealDB from a pod |
| `ingress` | the `<domain>` Ingress and certificate |
| `status` | what is live |

`infra/ship.sh dns plan|apply|undo` runs locally against the Cloudflare API (`CF_API_TOKEN` in
`~/.punchable/cloudflare.env`) and only touches the records it created.

## Files
| Path | What |
|---|---|
| `config.env.example` | the only place the hostnames, namespaces and IP are set |
| `render.sh` | fills `__PK_*__` placeholders in a manifest from `config.env`; refuses to output a leftover placeholder |
| `k8s/` | namespace, default-deny NetworkPolicies, SurrealDB StatefulSet, API and web Deployments, Ingress |
| `keycloak/` | the `punchable` realm (Google login, PKCE, short tokens), its import job, the login theme, realm tests |
| `web/` | nginx image for the exported web app |
| `dns/cf_dns.py` | Cloudflare A records for the domain and the auth host |
| `secrets.sh`, `db-init.sh` | generate the in-cluster Secrets; create the SurrealDB namespace, database and users |

Backups are not included: point your own backup job at a SurrealDB export (`surreal export`) and keep the audio
bucket versioned.
