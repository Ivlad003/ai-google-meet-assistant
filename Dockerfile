# =============================================================================
# Stage 1: Build Jarvis (Rust)
# Build on Ubuntu Jammy (same as runtime) to match glibc version (2.35)
# =============================================================================
FROM ubuntu:22.04 AS rust-builder

# Install Rust toolchain + whisper-rs-sys build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    build-essential \
    pkg-config \
    libssl-dev \
    cmake \
    clang \
    libclang-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /build

# Cache dependencies by copying manifests first
COPY jarvis/Cargo.toml jarvis/Cargo.lock* ./jarvis/

# Create a dummy main.rs to build dependencies
RUN mkdir -p jarvis/src && echo "fn main() {}" > jarvis/src/main.rs
RUN cd jarvis && cargo build --release 2>/dev/null || true

# Copy actual source and rebuild
COPY jarvis/ ./jarvis/
RUN touch jarvis/src/main.rs && cd jarvis && cargo build --release

# =============================================================================
# Stage 2: Build vexa-bot (TypeScript)
# =============================================================================
FROM node:20-bookworm AS ts-builder

WORKDIR /build/vexa-bot/core

# Cache npm dependencies
COPY services/vexa-bot/core/package.json services/vexa-bot/core/package-lock.json* ./
RUN npm install

# Copy source and build
COPY services/vexa-bot/core/ ./
RUN npm run build

# =============================================================================
# Stage 3: Runtime
# =============================================================================
FROM mcr.microsoft.com/playwright:v1.56.0-jammy AS runtime

# Install system dependencies (no fluxbox — not started, just dead weight)
RUN apt-get update && apt-get install -y --no-install-recommends \
    xvfb \
    ffmpeg \
    pulseaudio \
    x11-utils \
    x11-xserver-utils \
    jq \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy Jarvis binary
COPY --from=rust-builder /build/jarvis/target/release/jarvis /app/jarvis

# Copy vexa-bot
COPY --from=ts-builder /build/vexa-bot/core/dist/ /app/vexa-bot/core/dist/
COPY --from=ts-builder /build/vexa-bot/core/node_modules/ /app/vexa-bot/core/node_modules/
COPY --from=ts-builder /build/vexa-bot/core/package.json /app/vexa-bot/core/package.json
COPY --from=ts-builder /build/vexa-bot/core/build-browser-utils.js /app/vexa-bot/core/build-browser-utils.js

# Install Playwright browsers AFTER copying node_modules so the version matches the npm package
RUN cd /app/vexa-bot/core && npx playwright install --with-deps chromium \
    && rm -rf /tmp/*

# Create data directories with proper ownership for non-root user (pwuser = UID 1001)
RUN mkdir -p /data/jarvis/sessions /data/jarvis/logs /app/storage/screenshots /app/storage/logs /app/storage/temp \
    && chown -R pwuser:pwuser /data /app/storage /app/jarvis /app/vexa-bot /app/entrypoint.sh 2>/dev/null || true

# Copy entrypoint and default config
COPY docker/entrypoint.sh /app/entrypoint.sh
RUN chmod +x /app/entrypoint.sh \
    && chown -R pwuser:pwuser /app

COPY jarvis.config.example.json /app/default-config.json
RUN chown pwuser:pwuser /app/default-config.json

# Environment
ENV DISPLAY=:99
ENV DOCKER_MODE=1
ENV VEXA_BOT_DIR=/app/vexa-bot
ENV JARVIS_DATA_DIR=/data/jarvis
ENV JARVIS_CONFIG=/etc/jarvis/config.json
ENV HOME=/home/pwuser

# Bridge port 9090 is internal-only (Jarvis <-> vexa-bot within same container)
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -f http://localhost:8080/health && xdpyinfo -display :99 >/dev/null 2>&1 || exit 1

# Entrypoint runs as root to start Xvfb/PulseAudio, then drops to pwuser via gosu
RUN apt-get update && apt-get install -y --no-install-recommends gosu && rm -rf /var/lib/apt/lists/*

ENTRYPOINT ["/app/entrypoint.sh"]
