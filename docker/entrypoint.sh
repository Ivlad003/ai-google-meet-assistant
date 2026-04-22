#!/bin/bash
set -e

echo "[entrypoint] Starting Jarvis Docker environment..."

# --- Clean up stale lock files from previous runs ---
rm -f /tmp/.X99-lock /tmp/.X99-unix/X99 2>/dev/null || true

# --- Virtual Display ---
echo "[entrypoint] Starting Xvfb on :99..."
Xvfb :99 -screen 0 1280x720x24 -ac +extension GLX +render -noreset &

# Poll for Xvfb readiness instead of blind sleep
for i in $(seq 1 30); do
    xdpyinfo -display :99 >/dev/null 2>&1 && break
    sleep 0.2
done
if ! xdpyinfo -display :99 >/dev/null 2>&1; then
    echo "[entrypoint] ERROR: Xvfb failed to start on :99"
    exit 1
fi
echo "[entrypoint] Xvfb ready"

# --- PulseAudio ---
echo "[entrypoint] Starting PulseAudio daemon..."
# In Docker there's no login session, so use system mode with permissive access
export XDG_RUNTIME_DIR=/tmp/runtime-root
mkdir -p "$XDG_RUNTIME_DIR" /run/pulse /root/.config/pulse
# Allow all users to connect (needed for pwuser to use PulseAudio started by root)
echo "default-server = unix:/tmp/pulse-server" > /root/.config/pulse/client.conf
mkdir -p /home/pwuser/.config/pulse
echo "default-server = unix:/tmp/pulse-server" > /home/pwuser/.config/pulse/client.conf
chown -R pwuser:pwuser /home/pwuser/.config 2>/dev/null || true

pulseaudio --daemonize --system --disallow-exit --no-cpu-limit \
    --module-idle-time=-1 --exit-idle-time=-1 \
    -L "module-native-protocol-unix auth-anonymous=1 socket=/tmp/pulse-server" \
    2>/dev/null || \
pulseaudio --start --exit-idle-time=-1 2>/dev/null || true

# Poll for PulseAudio readiness
for i in $(seq 1 30); do
    pactl info >/dev/null 2>&1 && break
    sleep 0.2
done
if ! pactl info >/dev/null 2>&1; then
    echo "[entrypoint] WARNING: PulseAudio not ready, TTS audio injection may not work"
else
    echo "[entrypoint] PulseAudio ready"
fi

echo "[entrypoint] Creating PulseAudio null sink for audio capture..."
pactl load-module module-null-sink sink_name=meet_sink sink_properties=device.description="MeetAudioSink" 2>/dev/null || true

echo "[entrypoint] Creating PulseAudio TTS sink for voice agent..."
pactl load-module module-null-sink sink_name=tts_sink sink_properties=device.description="TTSAudioSink" 2>/dev/null || true

echo "[entrypoint] Creating virtual microphone from TTS sink monitor..."
pactl load-module module-remap-source master=tts_sink.monitor source_name=virtual_mic source_properties=device.description="VirtualMicrophone" 2>/dev/null || true
pactl set-default-source virtual_mic 2>/dev/null || true

# Configure ALSA to route through PulseAudio
mkdir -p "$HOME"
cat > "$HOME/.asoundrc" <<'ALSA_EOF'
pcm.!default {
    type pulse
}
ctl.!default {
    type pulse
}
ALSA_EOF

# --- Config File ---
# The mounted config may be read-only, so always work with a writable copy
MOUNTED_CONFIG="${JARVIS_CONFIG:-/etc/jarvis/config.json}"
RUNTIME_CONFIG="/tmp/jarvis-runtime-config.json"

if [ -f "$MOUNTED_CONFIG" ]; then
    cp "$MOUNTED_CONFIG" "$RUNTIME_CONFIG"
else
    echo "[entrypoint] No config file at $MOUNTED_CONFIG, using default..."
    cp /app/default-config.json "$RUNTIME_CONFIG"
fi

# Apply environment variable overrides via jq
if [ -n "$OPENAI_API_KEY" ]; then
    jq --arg v "$OPENAI_API_KEY" '.openai_key = $v' "$RUNTIME_CONFIG" > /tmp/config-tmp.json && mv /tmp/config-tmp.json "$RUNTIME_CONFIG"
fi
if [ -n "$BOT_NAME" ]; then
    jq --arg v "$BOT_NAME" '.bot_name = $v' "$RUNTIME_CONFIG" > /tmp/config-tmp.json && mv /tmp/config-tmp.json "$RUNTIME_CONFIG"
fi
if [ -n "$MEET_URL" ]; then
    jq --arg v "$MEET_URL" '.meet_url = $v' "$RUNTIME_CONFIG" > /tmp/config-tmp.json && mv /tmp/config-tmp.json "$RUNTIME_CONFIG"
fi
if [ -n "$LANGUAGE" ]; then
    jq --arg v "$LANGUAGE" '.language = $v' "$RUNTIME_CONFIG" > /tmp/config-tmp.json && mv /tmp/config-tmp.json "$RUNTIME_CONFIG"
fi
if [ -n "$TTS_VOICE" ]; then
    jq --arg v "$TTS_VOICE" '.tts_voice = $v' "$RUNTIME_CONFIG" > /tmp/config-tmp.json && mv /tmp/config-tmp.json "$RUNTIME_CONFIG"
fi
if [ -n "$OPENAI_MODEL" ]; then
    jq --arg v "$OPENAI_MODEL" '.openai_model = $v' "$RUNTIME_CONFIG" > /tmp/config-tmp.json && mv /tmp/config-tmp.json "$RUNTIME_CONFIG"
fi
if [ -n "$INTENT_MODEL" ]; then
    jq --arg v "$INTENT_MODEL" '.intent_model = $v' "$RUNTIME_CONFIG" > /tmp/config-tmp.json && mv /tmp/config-tmp.json "$RUNTIME_CONFIG"
fi
if [ -n "$RESPONSE_MODE" ]; then
    jq --arg v "$RESPONSE_MODE" '.response_mode = $v' "$RUNTIME_CONFIG" > /tmp/config-tmp.json && mv /tmp/config-tmp.json "$RUNTIME_CONFIG"
fi
if [ -n "$TRANSCRIPTION_MODE" ]; then
    jq --arg v "$TRANSCRIPTION_MODE" '.transcription_mode = $v' "$RUNTIME_CONFIG" > /tmp/config-tmp.json && mv /tmp/config-tmp.json "$RUNTIME_CONFIG"
fi
if [ -n "$MAX_RESPONSE_TOKENS" ]; then
    jq --argjson v "$MAX_RESPONSE_TOKENS" '.max_response_tokens = $v' "$RUNTIME_CONFIG" > /tmp/config-tmp.json && mv /tmp/config-tmp.json "$RUNTIME_CONFIG"
fi

# --- Ensure browser-utils bundle exists ---
BROWSER_UTILS="/app/vexa-bot/core/dist/browser-utils.global.js"
if [ ! -f "$BROWSER_UTILS" ]; then
    echo "[entrypoint] browser-utils.global.js missing; regenerating..."
    (cd /app/vexa-bot/core && node build-browser-utils.js) || echo "[entrypoint] Failed to regenerate browser-utils.global.js"
fi

# --- Fix permissions for volumes (may be created as root by Docker) ---
chown -R pwuser:pwuser /data/jarvis /app/storage /tmp/jarvis-runtime-config.json 2>/dev/null || true

echo "[entrypoint] Config loaded (mounted: $MOUNTED_CONFIG)"
# Ensure pwuser can access PulseAudio
export PULSE_SERVER=unix:/tmp/pulse-server

echo "[entrypoint] Starting Jarvis as pwuser..."

# Drop privileges: Xvfb/PulseAudio started as root above, now run Jarvis as pwuser
# gosu forwards signals (SIGTERM from docker stop) directly to Jarvis
exec gosu pwuser /app/jarvis --config "$RUNTIME_CONFIG"
