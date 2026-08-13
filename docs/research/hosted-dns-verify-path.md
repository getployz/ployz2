# Hosted DNS reachability probe contract

Research for [Verify the hosted DNS reachability probe contract](https://github.com/getployz/ployz2/issues/16).

Baseline: `psviderski/uncloud` `main` at `b7e224a1eff98813b1d1a32034d977be24be994e`. All line references are to that tree.

## Question

Does the hosted DNS service depend on, call, or otherwise constrain the public Caddy reachability
verification path, and may Ployz safely rename that path under its Ployz-owned identity rule?

## Answer

**No, and yes.** The hosted DNS service never sees the path. The probe is a purely client-side check
between two components this project reconstructs, so `/.uncloud-verify` is a Ployz-owned identifier
and renames to `/.ployz-verify` under the rule fixed in
[Choose the preserved, changed, and excluded product surface](https://github.com/getployz/ployz2/issues/7).

## Evidence

### The path is a constant shared by exactly two Uncloud components

- `internal/machine/caddyconfig/controller.go:22` — `VerifyPath = "/.uncloud-verify"`, the single
  definition.
- `internal/machine/caddyconfig/caddyfile.go:26` — the generated Caddyfile opens with a host-agnostic
  `http://` site block that handles `{{.VerifyPath}}` and responds with `{{.VerifyResponse}}` and
  status 200. The controller passes the machine ID as the response
  (`internal/machine/machine.go:473`).
- `pkg/client/dns.go:216` — `getVerifyURL` builds `http://<public-ip>/.uncloud-verify` by importing
  the same constant.

Both ends are reconstructed by this effort: the responder is the daemon's Caddyfile generator, the
caller is the CLI client. Nothing else in the tree reads the constant.

### The hosted DNS API surface never carries the path

`internal/dns/client.go` is the whole hosted-service client, and it makes exactly two calls against
`https://dns.uncloud.run/v1` (`cmd/uc/dns/reserve.go:18`):

| Call | Request | Response |
| --- | --- | --- |
| `POST {endpoint}/domains` | empty body, no auth | `{name, token}` |
| `POST {endpoint}/domains/{domain}/records` | `{name, type, values}` per record, `Authorization: Bearer <token>` | `{name, type, values, fqdn}` |

The record payload (`internal/dns/api.go`) carries a record name, an `A`/`AAAA` type, and IP strings.
There is no path, no probe URL, no challenge token, and no verification field anywhere in the request
or response types. The hosted service is told which IPs to publish and authenticates that instruction
with the bearer token issued at reservation; it is never told how the caller decided on those IPs.

### The probe is a client-side filter, not a service handshake

`pkg/client/dns.go:43` `CreateIngressRecords`:

1. Inspect the service (in practice `caddy`) and collect the machine IDs running its containers.
2. Drop machines with no `PublicIp`.
3. Concurrently `GET http://<public-ip>/.uncloud-verify` per remaining machine, with a 3s HTTP client
   timeout and exponential backoff capped at a 1s interval and 5s total elapsed
   (`pkg/client/dns.go:147`).
4. Accept a machine only on HTTP 200 **and** a response body exactly equal to that machine's ID. Any
   other status, body, or transport error reports
   `Unreachable (probably behind NAT or firewall)` and silently excludes the machine.
5. If no machine survives, return `ErrNoReachableMachines`; otherwise build one `A` record and/or one
   `AAAA` record named `*` from the survivors' IPs and call `CreateDomainRecords`.

The machine-ID equality check is what makes the probe meaningful: it proves the Caddy container of
*this* cluster answers on that public IP, rather than some unrelated web server. That check is
between the CLI and the daemon-generated Caddyfile — the hosted service plays no part.

### Direction of traffic confirms the split

- Reachability probe: CLI → machine public IP over plain HTTP on port 80, from wherever the operator
  runs `ployz`.
- Record creation: daemon (`internal/machine/cluster/dns.go:115` `CreateDomainRecords`) → hosted DNS
  API, since the daemon holds the reservation token.

The two never meet. The CLI only reaches the hosted service indirectly, through the daemon's gRPC
surface.

### Consumers of the probe

`UpdateDomainRecords` (`cmd/uc/caddy/deploy.go:196`) is the only caller of `CreateIngressRecords`, and
it is reached from `uc caddy deploy` and from `uc dns reserve` (`cmd/uc/dns/reserve.go:60`, which
calls it when a `caddy` service already exists). Machine `init`/`add` reach it through the same Caddy
deployment path.

## Residual unknown

Whether the hosted service *also* performs its own out-of-band reachability check on the submitted IPs
is not observable from this repository, and testing it would mean reserving a live domain against
Uncloud's production DNS service. The risk to this decision is nil either way: the service is never
given the path, so any server-side check it performs cannot be coupled to the path's name. Three
further signals point at there being no server-side check — record values are accepted verbatim, the
only authentication is the reservation token, and the client-side probe exists precisely because the
service publishes whatever it is handed.

## Consequences for the reconstruction

1. **Rename to `/.ployz-verify`.** It is a Ployz-owned identifier under issue #7 with no external
   constraint. Keep it defined once, in the Caddy config module, and have the client import that
   constant rather than restating the literal.
2. **The rename is user-visible.** `ployz caddy config` prints the generated Caddyfile, which contains
   the `handle /.ployz-verify` line, and upstream documents the block
   (`website/docs/3-concepts/2-ingress/2-publishing-services.md:228`,
   `website/docs/3-concepts/2-ingress/3-managing-caddy.md:196`). Record it in the deviation ledger
   from [Define the executable parity contract](https://github.com/getployz/ployz2/issues/11) as an
   expected difference, and keep the upstream Caddyfile fixtures normalized on it rather than
   asserting the Uncloud spelling.
3. **Preserve the Caddyfile block's shape and position.** The verification block is a bare `http://`
   site with no host matcher, emitted before the per-hostname sites, so a named `http://<hostname>`
   site takes precedence and the path is *not* served on published hostnames — only on requests
   addressed to the bare IP. That is exactly what the probe uses. A Rust generator that folds the
   verification handler into the hostname sites, or emits it after them, changes reachability
   semantics.
4. **Preserve the weak failure mode.** A machine that fails the probe is dropped from the DNS records
   with a progress line and no error; only an empty survivor set surfaces `ErrNoReachableMachines`
   with its remediation text. This is a partial result in the domain sense and must not grow retries,
   fencing, or a repair path.
5. **Freeze the path after the reconstruction.** The probe is an implicit contract between a CLI and
   whatever Caddyfile a possibly older `ployzd` generated. Changing the path later would make a newer
   CLI silently classify older machines as unreachable and drop them from ingress DNS. If it ever
   needs to change, it must go through the per-Machine capability mechanism from
   [Define RPC payload compatibility during Ployz version skew](https://github.com/getployz/ployz2/issues/15),
   not a straight edit. Within this reconstruction there is no skew, since the path is born as
   `/.ployz-verify` on both sides.

## Incidental finding: the JSON Caddy config is generated and never consumed

`internal/machine/caddyconfig/jsonconfig.go` builds a full `caddy.Config`, including its own copy of
the verification route (`jsonconfig.go:106`), and the controller writes it to `caddy.json` beside the
Caddyfile (`controller.go:251`). Nothing loads it:

- The Caddy container is started with `caddy run -c /config/Caddyfile`
  (`pkg/client/caddy.go:38`).
- The controller applies config by adapting the *Caddyfile* through the admin socket and POSTing the
  result to `/load` (`internal/machine/caddyconfig/client.go:51,94`).

So `caddy.json` is a written-only artifact with a duplicate definition of the verification route. It
is out of scope for this ticket, but it is a candidate for omission under the owned-code boundary from
[Choose technology bindings and owned-code boundaries](https://github.com/getployz/ployz2/issues/10) —
reconstructing it would mean porting a second config generator, with its own copy of this path, that
no component reads.
