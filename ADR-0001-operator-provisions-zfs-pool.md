# Operator provisions the Machine ZFS Pool

Deploy must not create a ZFS pool. An operator command creates the Machine ZFS Pool (size or adopted zpool); `machine init` may call that same command. Hidden auto-create on first `x-zfs` would pick a disk budget the operator never approved, and send/recv is out of scope so the pool stays machine-local on purpose.
