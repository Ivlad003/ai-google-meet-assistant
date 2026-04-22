---
name: docker-caddy-port-routing
description: Caddy reverse proxy must be the only externally-exposed port; internal services use Docker network only
triggers:
  - "8080 no auth"
  - "bypass caddy"
  - "localhost:8080 vs caddy"
  - "docker expose vs ports"
---

# Docker Caddy Port Routing — Don't Expose Internal Services

## The Insight
When using Caddy as a reverse proxy with auth in docker-compose, the internal service must NOT have its port mapped to the host. Otherwise users can bypass Caddy (and its auth) by hitting the service port directly.

Docker networking: `expose` makes a port reachable within the Docker network (Caddy -> Jarvis), while `ports` maps it to the host (accessible from outside).

## Why This Matters
If both Caddy (:8080 with auth) and Jarvis (:8080 direct) are mapped to the host, auth provides zero security — anyone can just use the direct port. This is especially dangerous on a VPS where all ports are publicly reachable.

## Recognition Pattern
- docker-compose has both a reverse proxy service AND the app service with `ports` mappings
- User can access the app without auth by hitting the app's port directly
- "I don't see any auth modal" when accessing the direct port

## The Approach
- **Internal service**: use `expose: ["8080"]` (Docker network only, no host binding)
- **Caddy**: use `ports: ["8080:8080"]` to be the sole entry point
- Caddy proxies to the service using its Docker network hostname (e.g. `reverse_proxy jarvis:8080`)
- For local dev, you can temporarily add `127.0.0.1:8080:8080` to the app for debugging

## Example
```yaml
services:
  app:
    expose:
      - "8080"  # Only reachable by Caddy inside Docker network

  caddy:
    ports:
      - "8080:8080"  # The ONLY externally-accessible port
    # reverse_proxy app:8080 in Caddyfile
```
