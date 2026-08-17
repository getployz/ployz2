# Cloud is the application plus a Cloud Relay; Machines dial out over tonic/h2

Ployz Cloud is one product in every environment: control plane + Cloud Relay. Machines Register with a long-lived HTTP/2 stream, Cloud Dials, the relay Opens on that control stream, the Machine Attaches a second outbound stream, and the inner bytes are the existing Machine RPC Channel. No Unix shortcut for Cloud. No yamux, no Quinn. TLS belongs to the HTTP/2 terminator in front (Caddy or a platform), not this binary.

Spec: https://github.com/getployz/ployz2/issues/297

## Considered Options

- **Inbound public Machine RPC / NATS** — fails behind NAT; this reconstruction has no inbound daemon authn.
- **Cloud as a WireGuard peer** — puts a multi-tenant process in Machine Subnet / Management Address space and needs outbound UDP.
- **Stored SSH** — Cloud holding customer SSH; already culled as legacy.
- **Unix-socket Cloud when colocated** — a second topology; self-host and dev would lie. Rejected.
- **Yamux (or Quinn) as the mux** — extra crate, and Quinn is UDP. HTTP/2 already multiplexes; the Machine is the client so Open-then-Attach is the stream dance.
- **Relay URL inside the bootstrap token** — freeze the hostname into pairing. Cloud writes `relay_urls` on the pairing row instead.
- **Daemon polls the control plane to discover relays** — the dialer would interpret Cloud. Policy-as-data: it dials stored URLs.
- **Sticky load balancer / two active acceptors** — attach can miss. One acceptor until scatter/forward.
- **Bundled cloudflared/frp** — third-party binary inside `ployzd`.
