---
name: docker-readonly-config-mount
description: Docker read-only volume mounts cannot be modified in-place by jq or sed — copy to /tmp first
triggers:
  - "mv: cannot move: Device or resource busy"
  - "read-only config volume"
  - "jq env var override Docker"
  - "entrypoint config override"
---

# Docker Read-Only Config Mount Override Pattern

## The Insight
When a config file is mounted as `:ro` in docker-compose, the entrypoint cannot modify it in place. `jq ... file > /tmp/x && mv /tmp/x file` fails because `mv` can't replace the read-only bind mount. The container crash-loops because `set -e` exits on the failed `mv`.

## Why This Matters
The common pattern of "mount config as read-only, override with env vars via jq" breaks silently. The container starts, runs Xvfb/PulseAudio, then dies on the config override step. With `restart: unless-stopped`, it crash-loops rapidly.

## Recognition Pattern
- docker-compose volume with `:ro` flag
- Entrypoint uses `jq` to merge env vars into config
- Error: `mv: cannot move '/tmp/config.json' to '/etc/jarvis/config.json': Device or resource busy`

## The Approach
Copy the mounted config to a writable location FIRST, apply overrides there, then point the app at the writable copy:

```bash
MOUNTED_CONFIG="/etc/app/config.json"      # Read-only mount
RUNTIME_CONFIG="/tmp/app-runtime-config.json"  # Writable copy

cp "$MOUNTED_CONFIG" "$RUNTIME_CONFIG"

# Apply overrides to the WRITABLE copy
if [ -n "$API_KEY" ]; then
    jq --arg v "$API_KEY" '.key = $v' "$RUNTIME_CONFIG" > /tmp/cfg-tmp.json && mv /tmp/cfg-tmp.json "$RUNTIME_CONFIG"
fi

exec /app/binary --config "$RUNTIME_CONFIG"
```
