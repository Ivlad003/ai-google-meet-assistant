#!/bin/sh
# Generate Caddyfile based on CADDY_AUTH_ENABLED env var
# Default: auth enabled if CADDY_AUTH_HASH is set

LISTEN="${CADDY_LISTEN:-:8080}"
UPSTREAM="jarvis:8080"

if [ "${CADDY_AUTH_ENABLED}" = "false" ] || [ -z "${CADDY_AUTH_HASH}" ]; then
    echo "[caddy-entrypoint] Auth DISABLED — no basic_auth"
    cat > /etc/caddy/Caddyfile <<EOF
${LISTEN} {
    reverse_proxy ${UPSTREAM}
}
EOF
else
    echo "[caddy-entrypoint] Auth ENABLED for user: ${CADDY_AUTH_USER:-admin}"
    cat > /etc/caddy/Caddyfile <<EOF
${LISTEN} {
    basic_auth {
        ${CADDY_AUTH_USER:-admin} ${CADDY_AUTH_HASH}
    }
    reverse_proxy ${UPSTREAM}
}
EOF
fi

echo "[caddy-entrypoint] Caddyfile generated:"
cat /etc/caddy/Caddyfile

exec caddy run --config /etc/caddy/Caddyfile --adapter caddyfile
