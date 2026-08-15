# Operator provisions the Machine ZFS Pool

Deploy must not create a ZFS pool. An operator command creates the Machine ZFS Pool (`--size 100G`, `--size 80%` of available backing-FS space resolved once, or `--from` an imported zpool) and may set an overcommit ratio on that pool. There is no stored “ZFS-enabled cluster”: capability is Live Observation per Machine. `machine init` is out of scope. Hidden auto-create on first `x-zfs` would pick a disk budget the operator never approved, and send/recv is out of scope so the pool stays machine-local on purpose.
