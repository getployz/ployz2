#!/usr/bin/env bash

set -uo pipefail

if [ "$#" -lt 2 ]; then
    echo "usage: $0 VERSION DESTINATION..." >&2
    exit 2
fi

version=$1
shift
ployz_bin=${PLOYZ_BIN:-ployz}
ssh_bin=${SSH_BIN:-ssh}
run=${PLOYZ_QUALIFY_RUN:-$(date -u +%Y%m%d%H%M%S)-$$}
total=$#
failures=0
index=0
diagnostics='systemctl status ployz.service --no-pager || true; journalctl -u ployz.service --no-pager -n 200 || true; docker inspect ployz-corrosion || true; docker logs --tail 200 ployz-corrosion || true; ss -H -lntup || true'

# OpenSSH needs -p PORT. user@host:2222 is a Ployz destination, not an OpenSSH one.
qualify_ssh() {
    local destination=$1
    shift
    local user=${destination%%@*}
    local rest=${destination#*@}
    local target=$destination
    local port=
    if [ "$user@$rest" = "$destination" ] && [ -n "$rest" ]; then
        case $rest in
            \[*:*)
                if [ "${rest#*\]:}" != "$rest" ]; then
                    port=${rest##*:}
                    target="$user@${rest%:*}"
                fi
                ;;
            *:*)
                local colons=${rest//[^:]/}
                if [ "${#colons}" -eq 1 ]; then
                    port=${rest##*:}
                    target="$user@${rest%:*}"
                fi
                ;;
        esac
        case $port in
            '' | *[!0-9]* | 0)
                port=
                target=$destination
                ;;
        esac
        if [ -n "$port" ] && [ "$port" -gt 65535 ]; then
            port=
            target=$destination
        fi
    fi
    if [ -n "$port" ]; then
        "$ssh_bin" -p "$port" "$target" "$@"
    else
        "$ssh_bin" "$target" "$@"
    fi
}

for destination in "$@"; do
    index=$((index + 1))
    echo "[$index/$total] initializing $destination"
    # No --yes: that flag confirms ResetRequest on an already-initialized Machine.
    if "$ployz_bin" machine init "$destination" \
        --context "qualify-$run-$index" \
        --name "qualify-$index" \
        --version "$version" \
        --storage none \
        --no-dns; then
        echo "[$index/$total] passed"
        continue
    fi

    failures=$((failures + 1))
    echo "[$index/$total] failed; collecting bind diagnostics" >&2
    qualify_ssh "$destination" "$diagnostics" || true
done

if [ "$failures" -ne 0 ]; then
    echo "$failures/$total clean founder initializations failed" >&2
    exit 1
fi

echo "$total/$total clean founder initializations passed"
