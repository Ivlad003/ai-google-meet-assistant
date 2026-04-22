---
name: docker-glibc-mismatch
description: Rust builder and runtime images must share the same glibc version in multi-stage Docker builds
triggers:
  - "GLIBC_2.39 not found"
  - "libc.so.6 version not found"
  - "rust:latest Dockerfile"
  - "playwright jammy glibc"
---

# Docker Multi-Stage GLIBC Mismatch

## The Insight
In multi-stage Docker builds, the builder image's glibc version determines the MINIMUM glibc the binary requires at runtime. If the builder has a newer glibc than the runtime, the binary silently links against unavailable symbols and crashes on startup with `GLIBC_X.XX not found`.

## Why This Matters
`rust:latest` tracks the latest Debian (currently Trixie, glibc 2.39). The `playwright:v1.56.x-jammy` runtime is Ubuntu 22.04 (glibc 2.35). A Rust binary compiled on Trixie WILL NOT run on Jammy. The error only appears at container startup — the build succeeds.

## Recognition Pattern
- Multi-stage Dockerfile with Rust builder + different runtime base
- Error: `/app/binary: /lib/.../libc.so.6: version 'GLIBC_X.XX' not found`
- Binary built fine, crashes immediately on exec

## The Approach
Match the builder OS to the runtime OS. For this project:
- Runtime: `mcr.microsoft.com/playwright:v1.56.x-jammy` (Ubuntu 22.04)
- Builder: `FROM ubuntu:22.04` + install Rust via rustup

Never use `rust:latest` or `rust:bookworm` when the runtime is Ubuntu Jammy. The 0.01 version difference in glibc (2.36 vs 2.35) is enough to break.

## Example
```dockerfile
# BAD: rust:latest = Debian Trixie (glibc 2.39), runtime = Jammy (glibc 2.35)
FROM rust:latest AS builder

# GOOD: Same OS as runtime
FROM ubuntu:22.04 AS builder
RUN apt-get update && apt-get install -y curl build-essential
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
```
