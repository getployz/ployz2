# Operator provisions the Machine ZFS Pool

There is no cluster zpool. `CreateZfsPool` is a Machine RPC. Default size is that Machine’s backing FS minus headroom `min(10GiB, 30% of FS total)`, capped by available, `fallocate`d so Docker/OS keep the headroom. `--size` / `--from` override. Quota is a packing claim enforced at Deploy: the Cluster entry observes declared quotas on each Machine and refuses if the new claim would not fit there. No `refreservation`. `machine init` is out of scope. Send/recv is out of scope so an existing volume is not moved to a emptier Machine.
