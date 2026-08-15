# Operator provisions the Machine ZFS Pool

There is no cluster zpool. `CreateZfsPool` is a Machine RPC (same targeting as `volume create`). Pool size is operator `--size` / `--from` on **that** Machine. Quota packing is 100% of that pool: the Cluster entry observes who has room, then `Ensure` commits on one Machine. No `refreservation`; used-bytes monitoring is later. Do not treat 100% packing as `--size 100%` of the host disk. `machine init` is out of scope. Send/recv is out of scope so an existing volume is not moved to a emptier Machine.
