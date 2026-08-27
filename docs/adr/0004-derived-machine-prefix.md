# Machine Prefix is derived from the WireGuard key; Allocator and Machine Subnet are deleted

Ployz keeps Uncloud `misc/design.md`: equal Machines, AP, operate each partition, heal later. It stops copying Uncloud's unique IPv4 `/24` pool and the Allocator Ployz added on top. A Machine's container space is a function of its own key, never a grant.

Containers are IPv4-only. The mesh is IPv6. eBPF on each Machine translates, and is the later monitoring attach point. That does not bring back a cluster IPv4 pool.

Status: proposed.

## Decision

Every cluster-routable identity is a pure function of the Machine's WireGuard public key, plus bits that Machine already has (its Local Container IPv4). Nothing cluster-routable is taken from a shared pool. Join and Deploy do not consult a coordinator.

The Service Container's interface is IPv4 only. Many images never open IPv6. Cross-Machine packets are IPv6 on `ployz-wg`. eBPF at the veth edge does the conversion. Internal DNS returns observer-local IPv4 Reach Addresses, never a remote Machine's Docker IPv4, never a cluster-unique IPv4.

```
TODAY                              AFTER

Allocator --> unique /24           key --> fdcd:Machine::/80
container IPv4 is cluster id       container IPv4 is local only
                                   eBPF: IPv4 in netns <-> IPv6 on mesh
```

## Unrepresentable

`CODING_STANDARDS.md`: a type cannot represent an illegal state. Two Machines holding the same Machine Subnet is a cluster-wide invariant no local type can hold. Delete the resource.

Deleted:

1. `MachineSubnet` and `Machine.subnet`.
2. The Cluster IPv4 pool (`10.210.0.0/16`), founder `--network`, `FoundingCluster.network`, `cluster.network`.
3. Cluster-unique `ContainerAddress` as IPv4. The IPv6 Container Address is derived, not allocated. A remote Local Container IPv4 is not a cluster identity.
4. Stored `Machine.management_address` and `ManagementAddressMismatch`.
5. `AllocatorRow`, `cluster.allocator`, quiet, steal, Register-forward.
6. `NotAllocator`, `AllocatorNotQuiet`, `NoFreeSubnet`, `InvalidClusterNetwork`.
7. `IsolationLocked` as a Register refusal.
8. WireGuard allowed-IPs of a peer IPv4 `/24`. Docker bridge IPAM that *is* the Machine Subnet.
9. Dual-stack in the container as a requirement. The netns is IPv4. IPv6 exists on the host and the mesh.
10. Cluster-wide IPv4 A records. An A answer is a Reach Address on this Machine only.

Same public key on Register is the existing Machine. Same Machine Name with a different key is Name Ambiguity.

## Addressing

```
  16           64                    32                 16
+------+------------------+--------------------+----------------+
| fdcd | Machine key[0..7]| Local Container IPv4|     zero      |
+------+------------------+--------------------+----------------+
       |<---- /80 route ---->|
              Machine Prefix              Container Address /128
```

`fdcc` + key[0..13] stays Management Address `/128`, not stored. `fdcd` is a disjoint marker so a container IPv6 cannot land on a Management Address.

| Address | Where it lives | Who picks it | Uniqueness |
|---|---|---|---|
| Local Container IPv4 | container netns | Docker on this Machine | this Machine only. Overlap across Machines is expected. |
| Reach Address | Internal DNS A on this observer | this Machine, from its observations | this observer only. Not replicated. |
| Machine Prefix | host routes, WG allowed-IPs | derived from public key | by construction |
| Container Address | IPv6 on the mesh, not in the netns | derived: prefix + Local Container IPv4 | by construction |
| Management Address | mesh management | derived from public key | by construction |

Embed the Local Container IPv4 in the IPv6 host bits so the receiving Machine can DNAT without a cluster table. The sender still needs a Reach Address: the app only dials IPv4.

A Machine Prefix is a function, not a record. Never stored, never a field on the wire.

## Routing

eBPF sits on the container veth (or cgroup). It is the IPv4/IPv6 edge. It is also the later flow-monitor attach point. It is not an Allocator: maps are rebuilt from this Machine's Local Container IPv4s plus Replicated Observations.

```
IPv4-only container X                 IPv4-only container Y
172.17.0.4                            172.17.0.5   (may equal X's IPv4; fine)
     |                                      ^
     |  dst = Reach Address                 |
     v  (A record, this Machine only)       |
  eBPF on A                                 eBPF on B
     |  src 172.17.0.4 -> fdcd:A:0.4        |  fdcd:B:0.5 -> 172.17.0.5
     |  dst handle    -> fdcd:B:0.5         |  fdcd:A:0.4 -> Y's Reach Address
     v                                      |              for A (IPv4 peer)
  ployz-wg  ---- IPv6 /80 cryptokey ---->  ployz-wg
              allowed-IPs: mgmt /128
                           + fdcd:B::/80
```

1. Same-Machine: eBPF hairpins IPv4 to the local veth. No WireGuard. One path, so later monitoring sees local flows too.
2. Cross-Machine: IPv6 on the wire. Kernel + WireGuard route the `/80`. eBPF does not replace cryptokey routing.
3. No transit: IPv6 ingress is accepted only for this Machine's prefix, then DNAT to a local veth.
4. Outbound internet IPv4 keeps Docker masquerade, unchanged.
5. Docker `ployz` bridge stays IPv4. It does not take a cluster `/24`. Host has IPv6 on `ployz-wg` and the eBPF programs.
6. Duplicate WG allowed-IPs still silently blackhole. Derived `/80`s cannot collide by concurrency.

`ployzd` owns the programs, maps, and DNS handles. `ployz` loses `--network`.

## Join and Deploy

1. Joining Machine generates its keypair. Prefix and Management Address exist before Register.
2. It mints its own Machine ID.
3. Register is an introduction. No CIDR walk, forward, steal, quiet, or minority refusal.
4. It publishes its own row. Observers derive `/80`s and install WG allowed-IPs when they first send (on-demand peers at large fleets).
5. Deploy creates the container. Docker assigns Local Container IPv4. Container Address is prefix + that IPv4. Nobody is asked. A failed create has no address to release.

## Internal DNS and Ingress

1. Internal DNS Answer is a TTL-zero **A** of Reach Addresses for observer-visible Serving Containers. No AAAA in the container path. No A that is some other Machine's Local Container IPv4.
2. Same-Machine Serving Containers still get a Reach Address (not a special native path in DNS), so eBPF sees every `.internal` flow.
3. Nearest DNS Selector orders Reach Addresses whose containers sit on this Machine first.
4. Caller Project matches the query source Local Container IPv4 to exactly one local Service Container.
5. Ingress Proxy is host-side: it may dial Container Address over IPv6, or a local Reach Address. Public Ingress on the public side is unchanged.

## Partition

A admits C; B admits D; both sides Deploy; heal.

Prefixes exist from keys. Reach Addresses on each side are that partition's observer-local handles. They are allowed to differ for the same remote container; they are not Cluster truth.

Heal unions Machine and container observations. `/80`s stay. Local Container IPv4s stay. Reach Address tables rebuild. No renumber.

Ugly: Name Ambiguity, Hostname Owner, DNS sets growing, Reach Addresses changing after heal (TTL 0).

Cannot happen: unique-`/24` repair, Allocator election.

Pre-release Machine Subnet Clusters reset and re-join. Greenfield lockstep `ployz`/`ployzd`.

## Monitoring

eBPF monitoring does not change the address model. It rides the same programs.

1. Attach point is already there: veth egress/ingress.
2. Maps already have Local Container IPv4, Reach Address, Machine Prefix, Container Address. Flow logs can name Project, Machine, Container ID from this Machine's observations.
3. Events stay Machine-local, like Ingress Access Event. They are not Cluster truth and not a control plane.
4. Do not wait for Hubble. Ship translate. Emit later.
5. A cluster-wide IPv4 identity for nicer logs is an Allocator. Correlate with derived IPv6 / Container ID instead.

## Glossary

**Delete:** Machine Subnet. Allocator.

**Rewrite:**

**Container Address**:
The IPv6 identity of one Service Container on the mesh: Machine Prefix plus that container's Local Container IPv4. The container netns does not hold it. Unique by construction.
_Avoid_: Local Container IPv4, Reach Address, allocated address

**Machine Gateway**:
The IPv4 gateway of this Machine's Docker bridge. Machine-local. Not cluster-routable.
_Avoid_: Management Address, allocated gateway

**Management Address**:
IPv6 management-plane address derived from the public key. Not stored beside the key.
_Avoid_: Container Address, Reach Address

**Machine ID**:
Minted by the Machine it identifies. Never granted.
_Avoid_: granted identity, admitted identity

**Internal DNS Answer**:
Observer-local, TTL-zero A of Reach Addresses from replicated healthy Service Container observations, optionally filtered by Membership Observations. Not Cluster truth.
_Avoid_: Cluster-wide IPv4, remote Local Container IPv4 as an A record

**Nearest DNS Selector**:
Orders Reach Addresses for Serving Containers on this Machine first.
_Avoid_: subnet locality

**New:**

**Machine Prefix**:
IPv6 `/80` derived from the public key (`fdcd:` + 8 key bytes). Route key for one Machine. Never stored, never granted.
_Avoid_: Machine Subnet, allocated prefix, namespace

**Local Container IPv4**:
IPv4 Docker assigns in this Machine's netns. Overlap across Machines is expected. The image's only address.
_Avoid_: Container Address, cluster-unique IPv4, overlap as a defect

**Reach Address**:
IPv4 Internal DNS returns on this Machine for one Serving Container. Observer-local. eBPF maps it to a Container Address. Not replicated.
_Avoid_: Local Container IPv4 of a remote Machine, cluster IPv4, VIP, allocated endpoint

## Extension

Any field or KV row that answers "which Machine may assign?" fails the deletion test. Prefix derivation depends only on the address format and `WireGuardPublicKey`.

Reach Address assignment inspects only this Machine's observations. It is not a Cluster free-list.

When tempted:

1. Need a new cluster-routable identity? Derive it from a key or a Local Container IPv4 this Machine already has.
2. Need locality? This Machine vs not. Not a shared IPv4 subnet.
3. Need to heal? Rebuild Reach Address tables. Do not renumber Local Container IPv4s.
4. Need uniqueness? Construction on the wire. Handles stay local.
5. Need a coordinator? Stop.
6. Need nicer IPv4 logs across Machines? Use Container ID / derived IPv6. Do not allocate cluster IPv4.

## Out of scope

1. ACLs and per-Project isolation (later policy on the same eBPF maps).
2. Service virtual IPs.
3. Replacing WireGuard.
4. Kubernetes CNI / cluster IPAM.
5. IPv6 in the container netns.
6. Cluster-wide IPv4 A records.
7. A monitoring control plane or shipped Hubble.

## Considered options

Rejected: unique IPv4 `/24`s (dead at 256 Machines; 10k is unrepresentable); dual-stack in the image (IPv4-only images are the requirement); AAAA to the container (those images never ask); Jool/nftables NAT64 plus a later second eBPF monitor (two datapaths; monitoring then has to infer translations); eBPF as a cluster IPAM (maps are local or they are an Allocator); hashing all containers into `100.64.0.0/10` as if unique (birthday collisions at fleet scale); embedding a globally unique IPv4 in DNS (the deleted class).

## Arena record

Base: candidate D (derived prefix, pool deleted, isolation lock and stored Management Address removed).

Grafted: candidate A's 64-bit `/80` and duplicate-key heal; candidate B's guardrails, no-transit, and eBPF as a local map not an ownership table; candidate C's maintainer checklist.

Changed after the arena: IPv4-only netns is required; eBPF is the veth translator and the future monitoring attach point; Internal DNS stays A, of Reach Addresses. Dropped: dual-stack in the container; "no eBPF for reachability."
