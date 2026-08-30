# Ployz

Ployz is the distributed deployment system behind Ployz Cloud. This glossary defines the language shared by Cloud, client adapters, and peer Machines; architectural rules live in [DESIGN.md](DESIGN.md).

## Core

**Ployz**:
The distributed deployment engine in this repository and the engine behind Ployz Cloud.
_Avoid_: Ployz2

**Adapter**:
A boundary that translates a product surface or input format into Ployz operations without creating different domain semantics.
_Avoid_: Separate orchestrator, alternate control plane

**Cloud**:
The hosted product surface through which users primarily drive Ployz. It is not a Cluster controller or source of runtime truth.
_Avoid_: Control plane, scheduler

**Organization**:
The Cloud administrative scope within which users and enrollment credentials exist.
_Avoid_: Cluster, Relay Tenant

**Cluster**:
The conceptual peer scope containing Machines and Cluster-scoped configuration. It has no authoritative entity or Cluster ID.
_Avoid_: Cluster Observation, control plane

**Cluster Observation**:
One Entry Machine's incomplete, potentially stale view of a Cluster.
_Avoid_: Cluster, Cluster truth

**Machine**:
A durable participant identity in a Cluster. Its own lifecycle and another Machine's membership judgment are separate facts.
_Avoid_: Node, host, member

**Machine ID**:
The durable opaque identity of one Machine, distinct from its mutable Machine Name.
_Avoid_: Machine Name, hostname

**Machine Name**:
A human-facing Machine selector that is best-effort unique but may become ambiguous.
_Avoid_: Machine ID, unique hostname

**Entry Machine**:
The Machine through which a client observes a Cluster and routes commands.
_Avoid_: Controller, leader, source of Cluster truth

**CLI Context**:
A named client configuration containing ordered Connections used to find an Entry Machine. It is not a Cluster identity.
_Avoid_: Cluster, authenticated session

**Connection**:
One configured route by which a client may reach a Machine.
_Avoid_: Machine, Cluster membership

## Observations

**Live Observation**:
Data obtained directly from a Machine at one point in time.
_Avoid_: Current truth, complete state

**Replicated Observation**:
Data read from the local eventually convergent replica.
_Avoid_: Live Observation, desired state, Cluster truth

**Membership Observation**:
One Machine's potentially incomplete or stale judgment that another Machine is unknown, up, suspect, or down.
_Avoid_: Machine lifecycle, authoritative liveness

**Partial Result**:
A command result containing successful values together with target-specific failures or omissions.
_Avoid_: Atomic failure, complete result

## Deployment

**Project**:
An observer-derived ownership namespace for Services and their resources.
_Avoid_: Compose Project, persisted workflow

**Compose Project**:
Compose input loaded by the Compose adapter for one command.
_Avoid_: Project, Cluster resource

**Service**:
An observer-derived grouping of Service Containers with one Qualified Service identity.
_Avoid_: Persisted workload, desired Service record

**Qualified Service**:
A Service's logical identity: Project Name plus Service Name, written `project/name`.
_Avoid_: Service Name, Service ID

**Service ID**:
An opaque deployment identity that survives updates to a Service.
_Avoid_: Qualified Service, grouping key

**Service Container**:
A managed Docker container observed as one running instance of a Service.
_Avoid_: Service record, canonical replica

**Container ID**:
The durable runtime identity of one managed Docker container.
_Avoid_: Container name, replica identity

**Resolved Service Spec**:
The exact Service configuration attached to a Service Container when it is created.
_Avoid_: Canonical Service spec, desired state

**Deploy**:
A bounded command attempt to calculate and execute work against a Cluster Observation.
_Avoid_: Deployment resource, durable workflow

**Serving Container**:
A Service Container eligible to receive traffic in one observer's view.
_Avoid_: Authoritative endpoint, replica identity

## Network and storage

**Management Address**:
The mesh address used to reach a Machine's management plane.
_Avoid_: Container Address, public endpoint

**Machine Subnet**:
The routed subnet a Machine uses for its containers.
_Avoid_: Globally allocated subnet, Cluster network

**Container Address**:
The address assigned to one container within its Machine Subnet.
_Avoid_: Management Address, globally unique address

**Docker Volume**:
A Machine-local Docker storage resource whose identity includes its Machine.
_Avoid_: Cluster volume, replicated volume

**Machine Pool**:
A storage budget on one storage-ready Machine from which Provisioned Volumes are created.
_Avoid_: Cluster pool, storage cluster

**Provisioned Volume**:
A size-bounded Docker Volume created from a Machine Pool.
_Avoid_: Replicated volume, storage class

## Discovery and ingress

**Internal DNS Answer**:
An observer-local answer derived from visible Serving Containers.
_Avoid_: Service registry record, Cluster-wide endpoint set

**Public Ingress**:
The public HTTP request path from DNS through a Machine's Ingress Proxy to a Serving Container.
_Avoid_: Global edge, single process

**Ingress Proxy**:
The Machine-local process that receives published HTTP traffic and routes it to Serving Containers.
_Avoid_: Public Ingress, global load balancer

**Ingress Hostname**:
The public HTTP hostname through which a Service is exposed.
_Avoid_: Service identity, empty assignment sentinel

**Cluster Domain**:
The DNS domain reserved for automatically assigned Ingress Hostnames in one Cluster.
_Avoid_: Organization domain, Machine hostname

## Cloud connectivity

**Cloud Pairing**:
The Cluster-scoped grant that lets Machines connect outward for Cloud access.
_Avoid_: Per-Machine login, daemon authorization

**Cloud Relay**:
The hosted opaque byte pipe through which Cloud reaches a Machine without an inbound route.
_Avoid_: Machine proxy, Cluster controller

**Pairing Credential**:
The bearer by which a Machine identifies its Cloud Pairing to a Cloud Relay.
_Avoid_: Dial Credential, per-Machine identity

**Dial Credential**:
The process-wide bearer by which Cloud authorizes relay operations.
_Avoid_: Pairing Credential, Cluster credential
