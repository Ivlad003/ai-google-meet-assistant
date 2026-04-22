---
name: caddy-docker-compose-bcrypt-escaping
description: Bcrypt hashes with $ must be escaped as $$ in docker-compose .env files for Caddy basic_auth
triggers:
  - "CADDY_AUTH_HASH"
  - "variable is not set"
  - "Defaulting to a blank string"
  - "bcrypt docker-compose"
  - "basicauth caddy env"
  - "basic_auth 401"
---

# Caddy + Docker Compose Bcrypt Hash Escaping

## The Insight
Three systems interact when using Caddy basic_auth with docker-compose environment variables, and each interprets `$` differently:
1. **docker-compose** treats `$VAR` as variable expansion in `.env` files
2. **Caddy** uses `{$VAR}` for its own env var substitution
3. **bcrypt hashes** contain literal `$` characters (e.g. `$2a$14$...`)

The result: a bcrypt hash like `$2a$14$abc...` in `.env` gets parsed by docker-compose as variable `$2a` (empty) + `$14` (empty) + `$abc...` (empty), producing an empty string.

## Why This Matters
Without proper escaping, Caddy receives an empty hash, and ALL authentication attempts fail with 401 — even correct credentials. The docker-compose warning `"The 'CHANGE_ME...' variable is not set"` is the telltale sign, but it's easy to miss among other startup output.

## Recognition Pattern
- Caddy basic_auth returns 401 for ALL requests including correct credentials
- docker-compose warns about variables "not set" / "Defaulting to blank string"
- The `.env` file has a bcrypt hash starting with `$2a$` or `$2b$`

## The Approach
1. Generate the hash inside the Caddy container: `docker exec caddy caddy hash-password --plaintext 'password'`
2. In `.env`, escape every `$` as `$$`: `CADDY_AUTH_HASH=$$2a$$14$$abc123...`
3. Also note: Caddy v2.10+ deprecates `basicauth` — use `basic_auth` (with underscore)

## Example
```bash
# .env file — WRONG (docker-compose eats the $)
CADDY_AUTH_HASH=$2a$14$kTmydnm...

# .env file — CORRECT (escaped for docker-compose)
CADDY_AUTH_HASH=$$2a$$14$$kTmydnm...
```
