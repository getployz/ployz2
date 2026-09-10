Tailcat is pinned to `91dc4979bd4ae88af6ae2c8bb549616de4bcaa5a`.
`build.sh` fetches that commit and applies `lifecycle.patch` before building.
The patch is local; it has not been accepted upstream. Keep it until an upstream
release provides equivalent bounded admission and idle peer cleanup.

The adapter sets `Server.MaxClients` and `Server.ClientIdleTimeout`. Tailcat
rejects new peers at the cap. Admissions that never authenticate and completed
TCP clients expire after one idle interval plus at most one sweep interval.
Repeated meows do not renew an idle admission. A running TCP handler pins its
peer, including silent streams; the adapter must keep the handler running until
its connection closes. Cleanup removes the Tailcat map, magicsock map, netstack
addresses and WireGuard peer through `Engine.SyncDevicePeer`. No endpoint restart
or helper peer registry is used. The optional cleanup mode rejects UDP handlers.
Concurrent meow handlers are bounded to 64; excess admissions retry naturally.

Rung 1: `./build.sh --test` runs `TestBoundedClientLifecycle` against a local DERP
fixture. It fills and expires unauthenticated admissions, checks the cap, attempts
three real wrong-PSK handshakes, churns successful real TCP clients, and exchanges
data on an established stream after each cleanup. The test checks retained
library and engine peer state. Churn uses an injected observation time; final
cleanup exercises the production sweep timer. Run `go test -race -run TestBoundedClientLifecycle .`
in the prepared upstream directory for race instrumentation.
