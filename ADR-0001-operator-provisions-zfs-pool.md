# Operator provisions the Machine ZFS Pool

There is no cluster zpool. `CreateZfsPool` is a Machine RPC. Default size is that Machine’s backing FS minus headroom, sparse (`truncate`, not `fallocate`). Usage is best-effort: Docker and ZFS compete for host blocks. Quota is a packing claim enforced only at Deploy against other claims on that Machine. No `refquota`, no `refreservation`. `--size` / `--from` override. `machine init` is out of scope. Send/recv is out of scope.
