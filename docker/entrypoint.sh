#!/bin/sh
set -eu

DATA_DIR=${DATA_DIR:-/var/lib/site-manager}
CADDY_ROOT=${CADDY_ROOT:-/etc/caddy}

mkdir -p "$DATA_DIR/sites" "$DATA_DIR/repos" "$CADDY_ROOT"

# Start Caddy (it will get its config from the app via reload)
# Create a minimal initial Caddyfile
if [ ! -f "$CADDY_ROOT/Caddyfile" ]; then
    cat <<'EOF' > "$CADDY_ROOT/Caddyfile"
{
    admin :2019
}
EOF
fi

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
