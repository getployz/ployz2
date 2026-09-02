# Internal DNS

On a participating Machine, Ployz runs an internal resolver on the Docker-bridge gateway, UDP/TCP port 53. Healthy Service Containers are reachable by service name from other containers. This is not `ployz dns` (hosted cluster domain).

## Sub-features

- `dns-gateway` resolver listens on the Machine subnet gateway (first usable address of the `/24`).
- `dns-service` a healthy replica answers its Service Name from another container on the mesh.
- `dns-health` names track healthy replicated containers (unhealthy/stopped replicas drop out).

## How to get to it (user POV)

User deploys more than one service, `ployz exec` into one, and looks up the other by name (or curls `http://otherservice/`). There is no `ployz dns-internal` command.

## Driving it with helpers

Preconditions:

- Participating Cluster with at least two healthy Service Containers (informing uses three Machines).

- **Lookup.** After [cluster-deploy.md](cluster-deploy.md) of two services, `helpers/drive.sh proof exec -T web -- nslookup data` (or `wget -qO- http://data/`). Proof: resolution or HTTP body from the peer, not from Docker's default embedded DNS alone on a single-compose network.
- **Rung.** Informing: `ployz/tests/internal_dns_cluster.rs::internal_dns_tracks_healthy_replicated_containers`. TSV rung 5 is `gap`.
- **Skip.** No participating Machine. Do not treat `ployz dns show` as this feature.

## Gotchas

- `ployz dns reserve` / `show` / `release` talk to `https://dns.uncloud.run/v1`. Wrong path.
- The sleep fixture is a single service. Internal DNS proof needs a name to resolve.
- Gateway bind is per Machine subnet. Two uninitialized daemons never start this resolver.
