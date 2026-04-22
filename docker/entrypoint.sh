#!/bin/bash
set -e

echo "[entrypoint] Starting Jarvis Docker environment..."

# --- Clean up stale lock files from previous runs ---
rm -f /tmp/.X99-lock /tmp/.X99-unix/X99

# --- Virtual Display ---
echo "[entrypoint] Starting Xvfb on :99..."
Xvfb :99 -screen 0 1920x1080x24 -ac +extension GLX +render -noreset &
sleep 1

# --- PulseAudio ---
echo "[entrypoint] Starting PulseAudio daemon..."
pulseaudio --start --log-target=syslog 2>/dev/null || true
sleep 1

echo "[entrypoint] Creating PulseAudio null sink for audio capture..."
pactl load-module module-null-sink sink_name=zoom_sink sink_properties=device.description="ZoomAudioSink" 2>/dev/null || true

echo "[entrypoint] Creating PulseAudio TTS sink for voice agent..."
pactl load-module module-null-sink sink_name=tts_sink sink_properties=device.description="TTSAudioSink" 2>/dev/null || true

echo "[entrypoint] Creating virtual microphone from TTS sink monitor..."
pactl load-module module-remap-source master=tts_sink.monitor source_name=virtual_mic source_properties=device.description="VirtualMicrophone" 2>/dev/null || true
pactl set-default-source virtual_mic 2>/dev/null || true

# Configure ALSA to route through PulseAudio
mkdir -p /root
cat > /root/.asoundrc <<'ALSA_EOF'
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

echo "[entrypoint] Config loaded (mounted: $MOUNTED_CONFIG)"
echo "[entrypoint] Starting Jarvis..."

# Exec replaces the shell with Jarvis — signals (SIGTERM from docker stop) go directly to it
exec /app/jarvis --config "$RUNTIME_CONFIG"
