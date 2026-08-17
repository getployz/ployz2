# Cloud reaches a private Cluster through a single-node Rust Cloud Relay

Hosted Cloud has no operator vantage and must not expose Machine RPC. Every paired Machine dials one active Rust Cloud Relay over TLS/443; Cloud and the CLI attach to that same process; the relay splices opaque streams and holds no Cluster observation. One node until about 20k Machines — then the same pairing URL can grow to internal scatter/forward without changing `ployzd` or the SDK.

Spec: https://github.com/getployz/ployz2/issues/297

## Considered Options

- **Inbound public Machine RPC / NATS** — fails behind NAT; this reconstruction has no inbound daemon authn.
- **Cloud as a WireGuard peer** — puts a multi-tenant hosted process in Machine Subnet / Management Address space and needs outbound UDP.
- **Stored SSH** — Cloud holding customer SSH; already culled as legacy.
- **Sticky load balancer** — rejected: affinity breaks on topology change.
- **Elixir BEAM scatter/forward** — right *later* shape for any-node attach on a private network; not v1. Relays stay cattle; drain still drops sockets.
- **Bundled cloudflared/frp** — third-party binary inside `ployzd` with its own upgrade clock.
