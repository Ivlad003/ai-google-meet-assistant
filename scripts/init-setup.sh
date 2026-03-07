#!/bin/sh
set -e

ADMIN_URL="http://admin-api:8001"
GATEWAY_URL="http://api-gateway:8000"
SHARED="/shared"
MARKER="$SHARED/.setup-done"
KEY_FILE="$SHARED/api-key"

log() {
  echo "[init-setup] $*"
}

# --- Step 1: Small delay for postgres ---
log "Starting..."
sleep 2

# --- Step 2: Wait for admin-api healthy (max 60s) ---
log "Waiting for admin-api to be healthy..."
elapsed=0
while [ "$elapsed" -lt 60 ]; do
  if curl -sf "$ADMIN_URL/docs" > /dev/null 2>&1; then
    log "admin-api is healthy."
    break
  fi
  sleep 2
  elapsed=$((elapsed + 2))
done
if [ "$elapsed" -ge 60 ]; then
  log "ERROR: admin-api not healthy after 60s"
  exit 1
fi

# --- Step 3-6: Create user + API key (first boot only) ---
if [ -f "$MARKER" ] && [ -f "$KEY_FILE" ]; then
  log "Setup already done, skipping user/key creation."
else
  log "First boot — creating user and API key..."

  # Step 4: Create user
  USER_RESP=$(curl -sf -X POST "$ADMIN_URL/admin/users" \
    -H "X-Admin-API-Key: $ADMIN_API_TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"email":"bot@meet-assistant.ai","name":"Meet Bot"}' 2>&1) || {
    # User might already exist from a partial previous run
    log "User creation response: $USER_RESP"
    USER_RESP=$(curl -sf "$ADMIN_URL/admin/users/email/bot@meet-assistant.ai" \
      -H "X-Admin-API-Key: $ADMIN_API_TOKEN" 2>&1) || {
      log "ERROR: Cannot create or find user"
      exit 1
    }
  }

  USER_ID=$(echo "$USER_RESP" | sed -n 's/.*"id":\([0-9]*\).*/\1/p' | head -1)
  if [ -z "$USER_ID" ]; then
    log "ERROR: Could not extract user ID from: $USER_RESP"
    exit 1
  fi
  log "User ID: $USER_ID"

  # Step 5: Create API token
  TOKEN_RESP=$(curl -sf -X POST "$ADMIN_URL/admin/users/$USER_ID/tokens" \
    -H "X-Admin-API-Key: $ADMIN_API_TOKEN" \
    -H "Content-Type: application/json")
  API_KEY=$(echo "$TOKEN_RESP" | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')

  if [ -z "$API_KEY" ]; then
    log "ERROR: Could not extract token from: $TOKEN_RESP"
    exit 1
  fi

  # Step 6: Write token and marker
  echo "$API_KEY" > "$KEY_FILE"
  touch "$MARKER"
  log "API key saved to $KEY_FILE"
fi

API_KEY=$(cat "$KEY_FILE")

# --- Step 7: Parse MEET_URL ---
if [ -z "$MEET_URL" ]; then
  log "No MEET_URL set — skipping bot launch. Set it via web UI at http://localhost:8080"
  exit 0
fi

# Parse platform from URL
case "$MEET_URL" in
  *meet.google.com*)
    PLATFORM="google_meet"
    MEETING_ID=$(echo "$MEET_URL" | sed -n 's|.*meet\.google\.com/\([a-z0-9-]*\).*|\1|p')
    ;;
  *teams.microsoft.com*|*teams.live.com*)
    PLATFORM="msteams"
    MEETING_ID=$(echo "$MEET_URL" | sed 's|.*teams\.[^/]*/||')
    ;;
  *zoom.us*)
    PLATFORM="zoom"
    MEETING_ID=$(echo "$MEET_URL" | sed -n 's|.*zoom\.us/j/\([0-9]*\).*|\1|p')
    ;;
  *)
    log "ERROR: Unrecognized meeting URL: $MEET_URL"
    exit 1
    ;;
esac

if [ -z "$MEETING_ID" ]; then
  log "ERROR: Could not parse meeting ID from: $MEET_URL"
  exit 1
fi

log "Platform: $PLATFORM, Meeting ID: $MEETING_ID"

BOT_DISPLAY_NAME="${BOT_DISPLAY_NAME:-AI Assistant}"

# --- Step 8: Wait for api-gateway healthy (max 60s) ---
log "Waiting for api-gateway to be healthy..."
elapsed=0
while [ "$elapsed" -lt 60 ]; do
  if curl -sf "$GATEWAY_URL/docs" > /dev/null 2>&1; then
    log "api-gateway is healthy."
    break
  fi
  sleep 2
  elapsed=$((elapsed + 2))
done
if [ "$elapsed" -ge 60 ]; then
  log "ERROR: api-gateway not healthy after 60s"
  exit 1
fi

# --- Step 9: Launch bot into meeting ---
log "Launching bot into meeting..."
LAUNCH_RESP=$(curl -sf -X POST "$GATEWAY_URL/bots" \
  -H "X-API-Key: $API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"platform\":\"$PLATFORM\",\"native_meeting_id\":\"$MEETING_ID\",\"bot_name\":\"$BOT_DISPLAY_NAME\"}" 2>&1) || true

log "Launch response: $LAUNCH_RESP"

# --- Step 10: Done ---
log "Done! Web UI at http://localhost:8080"
exit 0
