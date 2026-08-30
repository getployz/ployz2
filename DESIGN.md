# Ployz design

Ployz is the distributed deployment engine behind Ployz Cloud and its web UI. Cloud is the primary product surface, while the CLI, SDK, and Compose integration are adapters into the same system.

Ployz runs containerized applications across user-owned Docker Machines. A Machine may be a cloud server, a small device, or anything in between. Machines form a peer Cluster so users can add compute without operating a separate control plane.

The main design problems are:

1. Connecting Machines and their containers.
2. Sharing observations without creating an authoritative controller.
3. Coordinating bounded deployment operations.
4. Discovering and exposing Services.
5. Letting Cloud drive the system without owning its availability.

## Cluster of Machines

A Cluster is the conceptual scope of peer Machines and Cluster-scoped configuration. It has no controller, leader, or authoritative Cluster object.

A client enters through one reachable Entry Machine. That Machine represents only what it can currently observe: membership and replicated data may be incomplete, stale, or contradictory. Another Entry Machine may produce a different Cluster Observation.

Commands operate on reachable Machines and preserve target-specific failures and omissions. A network partition therefore produces explicit partial results rather than false claims of Cluster-wide completeness. Replicated observations converge after connectivity returns; completed imperative operations are not automatically rolled back or reconciled.

## Network

Machines communicate over a WireGuard mesh. Each Machine has a durable identity and key, a management address, and a routed subnet for its containers. Containers can communicate across Machines without application-level proxies or address translation.

Each Machine advertises possible network endpoints. Endpoint selection is observer-local, so different Machines may choose different paths to the same peer as network conditions change.

The mesh is infrastructure for the Cluster, not a control plane. Losing one Machine does not remove a central coordinator because none exists.

## Shared observations

Each Machine owns its local identity, lifecycle, secrets, and runtime mutations. Docker is authoritative for live containers and volumes on that Machine.

Machines publish observations of their own resources into an eventually convergent replicated store provided by Corrosion. Replication makes those observations available to peers; it does not turn them into global truth or persisted desired state. Cluster-scoped values use narrow coordination rules specific to the value instead of a general leader.

This distinction is fundamental:

- A **Live Observation** comes directly from a Machine and can become stale immediately.
- A **Replicated Observation** comes from the local replica and may lag or conflict.
- A **Cluster Observation** is the incomplete view assembled by one Entry Machine.

## Orchestration

Ployz does not submit desired state to a central scheduler. A client gathers observations through an Entry Machine, calculates bounded work, presents the exact preview when confirmation is required, and invokes the selected Machines directly.

```text
Cloud UI / CLI / SDK
          |
      Ployz client
          |
    Entry Machine
       /      \
 Machine    Machine
    |          |
  Docker     Docker
```

A Deploy is an ephemeral command attempt, not a durable workflow or Cluster resource. Its plan may complete only a prefix. Failures report completed, failed, and unattempted work; there is no general transaction or rollback.

Projects and Services are derived from observed containers and volumes rather than persisted as canonical Cluster records. A Service Container carries the exact resolved specification used to create it, so containers observed in one Service may legitimately differ.

### Client and daemon responsibilities

Client changes are cheaper to distribute than daemon changes, so behavior should be designed for the client first. This is an economic preference, not a rule that all coordination belongs there.

The daemon owns behavior that must continue without a client, enforce a local safety boundary, or manage a local resource. It remains responsible for Machine lifecycle, networking, local Docker operations and observations, and Machine-local serving infrastructure. New daemon policy needs one of those reasons.

## Service discovery

Each Machine exposes internal DNS derived from its local replicated observations. Service names resolve to observed Serving Containers, and answers may differ between Machines or remain stale during a partition.

Service discovery is observer-relative. An authoritative DNS response for the internal zone is not proof of authoritative Cluster state or current network reachability.

## Ingress

Public Ingress is the request path from public DNS through a Machine-local Ingress Proxy to an observed Serving Container. It is not a single global edge or controller.

Ingress configuration and certificate material are derived asynchronously from Cluster observations. A completed configuration handoff does not prove that the proxy adopted it or that every upstream is reachable. During partitions, stale observations may cause requests to fail.

## Cloud

Cloud is the primary way users drive Ployz. It provides the web UI and coordinates enrollment, hosted DNS, and optional relay access to Machines that cannot accept inbound connections.

Cloud is not the Cluster control plane. The relay carries opaque streams, and Cloud does not own runtime truth, scheduling, or Cluster availability. Existing Machines can continue operating when Cloud is unreachable, although Cloud-driven actions and hosted services may be unavailable.

## Boundaries

Ployz deliberately does not provide:

- an authoritative global Cluster view;
- a centralized scheduler or control-plane quorum;
- Cluster-wide atomic operations or general rollback;
- automatic correction of every difference between observations;
- a generic abstraction over container runtimes; or
- replicated persistent storage.

These boundaries keep failure visible and let each Machine remain independently useful. Add a stronger guarantee only when the product requires it and the system can prove it.
