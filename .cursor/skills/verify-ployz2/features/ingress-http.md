# Ingress HTTP

App services become HTTP (or HTTPS) on the Cluster Ingress Proxy. Compose uses `x-ports` like `80/http` or `api.example.com:80/http`, or `x-caddy` with a Caddyfile. The proxy itself is `ployz ingress deploy` / founding `--ingress-backend`.

## Sub-features

- `ingress-ports` Compose `x-ports` (`80/http`, hostname:port/protocol). Cannot mix Compose `ports:` and `x-ports`. Cannot mix `x-caddy` and `x-ports`.
- `ingress-caddy` service `x-caddy: Caddyfile` or inline config.
- `ingress-proxy` `ployz ingress deploy` (`--image`, `-m`, `--recreate`, `--skip-health`). `ployz ingress config` prints the selected backend config as plain text.
- `ingress-backend` founding `--ingress-backend caddy|zentinel|envoy` (clap default `caddy`). Zentinel uses host networking.
- `ingress-curl` after Deploy, HTTP GET to the hostname reaches the service.

## How to get to it (user POV)

User adds `x-ports: ["80/http"]` (or a hostname) to a service, deploys, and hits the Ingress Proxy. `ployz ingress config` shows what the proxy loaded.

## Driving it with helpers

Preconditions:

- Participating Cluster. Prefer three Machines for the informing test; one Machine can still load Caddy.

- **Deploy a hostname.** Compose with `x-ports: ["80/http"]` or an explicit hostname, `helpers/drive.sh proof deploy --yes -f <file>`.
- **Config.** `helpers/drive.sh proof ingress config`. Stdout is the backend file, not coloured.
- **Fetch.** `curl` the published HTTP hostname from a place that can reach the Machine. Proof is status 200 and a body from the service.
- **Rung.** Informing: `ployz/tests/ingress_cluster.rs::caddy_projects_and_loads_cluster_services_on_three_machines` (zentinel/envoy modules in the same crate). TSV rung 5 is `gap`.
- **Skip.** No participating Machine, or no reachable HTTP port from this VM. Nested Docker may publish ports that this VM cannot route.

## Gotchas

- `ployz dns` hosted domain is not this path. Internal service DNS is [internal-dns.md](internal-dns.md). Certificates are [acme-certs.md](acme-certs.md).
- Deviation `plain-ingress-config` dropped `--no-color`.
- Deviation `ingress-proxy-vocabulary` replaced `caddy` / `--no-caddy` with `ingress` / `--no-ingress`.
