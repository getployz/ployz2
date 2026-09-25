# Build Order prototype (throwaway)

A snapshot of the 2026-09-25 exploration. Open any HTML file in a browser; each one stands alone.

| File | What it explored |
|---|---|
| `v1-variants.html` | Where "Build on" lives: service panel, Servers page, or deploy bar (`?variant=A\|B\|C`) |
| `v2-pool.html` | Ordered pool of servers and GitHub, plus a per-service override |
| `v3-lists.html` | Concurrency, "start within" timeouts, override as a list, no predictions |
| `v4-servers-entry.html` | "Your servers" as one entry; picks a server by warm cache, then free slots |
| `v5-per-repo.html` | GitHub first, with workflow status per repository |

The prototypes predate these later decisions. Where they differ, the spec wins:

- Build Order is a single select, not an ordered list.
- Cloud runs each Image Build as its own step, starting at admission. Deployment only delivers.
- Fall-through works in both directions.
- The UI shows no predictions: the rule before, live state during, evidence after.
