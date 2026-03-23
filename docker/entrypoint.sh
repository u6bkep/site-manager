#!/bin/sh
set -eu

DATA_DIR=${DATA_DIR:-/var/lib/site-manager}
CADDY_ROOT=${CADDY_ROOT:-/etc/caddy}

mkdir -p "$DATA_DIR/sites" "$DATA_DIR/repos" "$CADDY_ROOT"

# Always write a minimal Caddyfile so Caddy starts clean.
# The app will overwrite this with the real config on startup,
# and Caddy's --watch flag will pick up the change.
cat <<'EOF' > "$CADDY_ROOT/Caddyfile"
{
    admin :2019
}
EOF

echo "starting caddy..."
/usr/bin/caddy run --config "$CADDY_ROOT/Caddyfile" --adapter caddyfile --watch &
CADDY_PID=$!

cleanup() {
    echo "shutting down..."
    kill "$CADDY_PID" 2>/dev/null || true
    wait "$CADDY_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "starting site-manager..."
exec /usr/local/bin/site-manager "$@"
