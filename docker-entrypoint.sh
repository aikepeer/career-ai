#!/bin/sh
set -e

# careerai's dashboard is a separate process from the daemon. When the
# container runs the daemon (the default), also start the read-only
# dashboard in the background so the published port 3000 is reachable.
# It binds 0.0.0.0, which CAREERAI_DASHBOARD_ALLOW_NON_LOOPBACK=1 (set
# in the image) permits. See docs/DOCKER.md.
if [ "$1" = "daemon" ] || [ "$#" -eq 0 ]; then
    port="${CAREERAI_DASHBOARD_PORT:-3000}"
    (
        # Wait for the daemon to create the SQLite DB on a fresh volume
        # so the dashboard's read-only queries don't race migrations.
        i=0
        while [ "$i" -lt 60 ] && [ ! -f "${CAREERAI_ROOT}/data/careerai.sqlite" ]; do
            i=$((i + 1))
            sleep 1
        done
        exec careerai status serve --port "$port" --bind 0.0.0.0
    ) &
fi

# The daemon becomes PID 1 and receives SIGTERM for graceful shutdown.
# For one-shot subcommands (discover, match, tailor, ...) $@ is passed
# straight through without starting the dashboard.
exec careerai "$@"
