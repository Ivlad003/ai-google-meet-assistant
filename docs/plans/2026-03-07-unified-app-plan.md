# Unified App Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Merge Vexa + AI Meet Assistant into a single repo with one `docker compose up` command, auto-setup, and a web UI dashboard.

**Architecture:** Vexa services vendored into `services/` and `libs/`. A shell-based init container auto-creates the Vexa user + API key on first boot and launches the meeting bot. The meet-bot Go binary serves a web UI on port 8080 with config management, bot controls, and live transcript streaming.

**Tech Stack:** Go 1.24, Docker Compose, vanilla HTML/CSS/JS (go:embed), Vexa platform (Python services), PostgreSQL, Redis, MinIO.

---

### Task 1: Vendor Vexa Services

**Files:**
- Copy: `services/` (from Vexa repo, all subdirectories)
- Copy: `libs/shared-models/` (from Vexa repo)
- Copy: `alembic.ini` (from Vexa repo)
- Create: `.gitignore`

**Step 1: Copy Vexa directories**

```bash
cd /Users/kosmodev/Documents/pet_project
# Copy services (excluding 1.5GB model files)
cp -r vexa/services ai_google_meet_asistant/services
rm -rf ai_google_meet_asistant/services/transcription-service/models

# Copy shared libs
cp -r vexa/libs ai_google_meet_asistant/libs

# Copy alembic config
cp vexa/alembic.ini ai_google_meet_asistant/alembic.ini
```

**Step 2: Create .gitignore**

```gitignore
# Vexa model files (downloaded at build time)
services/transcription-service/models/

# Docker volumes
postgres-data/
redis-data/
minio-data/

# Environment
.env

# Old artifacts
*.png
models/
repomix-output.xml

# Go
/bot
```

**Step 3: Remove old artifacts**

```bash
cd /Users/kosmodev/Documents/pet_project/ai_google_meet_asistant
rm -f join-failed.png login-email-failed.png login-page.png meet-loaded.png repomix-output.xml
rm -rf models/
```

**Step 4: Verify structure**

```bash
ls services/admin-api services/api-gateway services/bot-manager services/transcription-collector services/tts-service services/vexa-bot services/WhisperLive services/transcription-service libs/shared-models alembic.ini
```

Expected: all directories exist, no errors.

**Step 5: Commit**

```bash
git add -A
git commit -m "feat: vendor Vexa services into repo"
```

---

### Task 2: Create Unified docker-compose.yml

**Files:**
- Replace: `docker-compose.yml`

**Step 1: Write the unified compose file**

Replace the current `docker-compose.yml` with this complete file. It merges Vexa's `docker-compose.yml` + `docker-compose.local-db.yml` + the meet-bot service + a new init-setup service. Key changes from Vexa's original:
- Postgres inlined (was separate local-db file)
- `whisperlive-remote` is the default (no profiles — always starts)
- GPU/CPU whisperlive variants removed (use remote transcription only)
- `mcp` service removed (not needed for meet assistant)
- `vexa-network` external network removed (self-contained)
- All build contexts changed from `.` to use `services/` paths directly
- `init-setup` and `meet-bot` added
- `setup-data` shared volume added
- Healthchecks added to admin-api and api-gateway

```yaml
name: ai-meet-assistant

services:
  # --- Infrastructure ---
  postgres:
    image: postgres:15-alpine
    environment:
      - POSTGRES_DB=vexa
      - POSTGRES_USER=postgres
      - POSTGRES_PASSWORD=postgres
    volumes:
      - postgres-data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres -d vexa"]
      interval: 5s
      timeout: 5s
      retries: 5
    networks:
      - default
    restart: unless-stopped

  redis:
    image: redis:7.0-alpine
    command: ["redis-server", "--appendonly", "yes", "--appendfsync", "everysec"]
    volumes:
      - redis-data:/data
    networks:
      - default
    restart: unless-stopped

  minio:
    image: minio/minio:latest
    command: server /data --console-address ":9001"
    environment:
      - MINIO_ROOT_USER=vexa-access-key
      - MINIO_ROOT_PASSWORD=vexa-secret-key
    volumes:
      - minio-data:/data
    networks:
      - default
    restart: unless-stopped

  minio-init:
    image: minio/mc:latest
    depends_on:
      - minio
    entrypoint:
      - /bin/sh
      - -c
      - |
        sleep 5
        mc alias set vexa http://minio:9000 vexa-access-key vexa-secret-key
        mc mb --ignore-existing vexa/vexa-recordings
        exit 0
    networks:
      - default

  # --- Vexa Core ---
  admin-api:
    build:
      context: .
      dockerfile: services/admin-api/Dockerfile
    environment:
      - DB_HOST=postgres
      - DB_PORT=5432
      - DB_NAME=vexa
      - DB_USER=postgres
      - DB_PASSWORD=postgres
      - DB_SSL_MODE=disable
      - ADMIN_API_TOKEN=auto-generated-admin-token
      - LOG_LEVEL=INFO
    healthcheck:
      test: ["CMD", "python", "-c", "import urllib.request; urllib.request.urlopen('http://localhost:8001/health')"]
      interval: 5s
      timeout: 5s
      retries: 10
      start_period: 10s
    depends_on:
      postgres:
        condition: service_healthy
    networks:
      - default
    restart: unless-stopped

  api-gateway:
    build:
      context: .
      dockerfile: services/api-gateway/Dockerfile
    ports:
      - "${API_GATEWAY_PORT:-8056}:8000"
    environment:
      - ADMIN_API_URL=http://admin-api:8001
      - BOT_MANAGER_URL=http://bot-manager:8080
      - TRANSCRIPTION_COLLECTOR_URL=http://transcription-collector:8000
      - MCP_URL=http://mcp:18888
      - REDIS_URL=redis://redis:6379/0
      - LOG_LEVEL=INFO
    healthcheck:
      test: ["CMD", "python", "-c", "import urllib.request; urllib.request.urlopen('http://localhost:8000/health')"]
      interval: 5s
      timeout: 5s
      retries: 10
      start_period: 10s
    depends_on:
      admin-api:
        condition: service_healthy
      bot-manager:
        condition: service_started
      transcription-collector:
        condition: service_started
    networks:
      - default
    restart: unless-stopped

  bot-manager:
    build:
      context: .
      dockerfile: services/bot-manager/Dockerfile
    environment:
      - REDIS_URL=redis://redis:6379/0
      - BOT_IMAGE_NAME=vexa-bot:dev
      - TTS_SERVICE_URL=http://tts-service:8002
      - DOCKER_NETWORK=ai-meet-assistant_default
      - LOG_LEVEL=INFO
      - DB_HOST=postgres
      - DB_PORT=5432
      - DB_NAME=vexa
      - DB_USER=postgres
      - DB_PASSWORD=postgres
      - DB_SSL_MODE=disable
      - DOCKER_HOST=unix://var/run/docker.sock
      - DEVICE_TYPE=remote
      - WHISPER_LIVE_URL=ws://whisperlive-remote:9090/ws
      - ADMIN_TOKEN=auto-generated-admin-token
      - STORAGE_BACKEND=minio
      - MINIO_ENDPOINT=minio:9000
      - MINIO_ACCESS_KEY=vexa-access-key
      - MINIO_SECRET_KEY=vexa-secret-key
      - MINIO_BUCKET=vexa-recordings
      - MINIO_SECURE=false
      - RECORDING_ENABLED=false
      - CAPTURE_MODES=audio
      - OPENAI_API_KEY=${OPENAI_API_KEY:-}
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
    depends_on:
      redis:
        condition: service_started
      tts-service:
        condition: service_started
      postgres:
        condition: service_healthy
    networks:
      - default
    restart: unless-stopped

  transcription-collector:
    build:
      context: .
      dockerfile: services/transcription-collector/Dockerfile
    volumes:
      - ./alembic.ini:/app/alembic.ini
      - ./libs/shared-models/alembic:/app/alembic
    environment:
      - DB_HOST=postgres
      - DB_PORT=5432
      - DB_NAME=vexa
      - DB_USER=postgres
      - DB_PASSWORD=postgres
      - DB_SSL_MODE=disable
      - REDIS_HOST=redis
      - REDIS_PORT=6379
      - REDIS_STREAM_NAME=transcription_segments
      - REDIS_CONSUMER_GROUP=collector_group
      - REDIS_STREAM_READ_COUNT=10
      - REDIS_STREAM_BLOCK_MS=2000
      - ADMIN_TOKEN=auto-generated-admin-token
      - BACKGROUND_TASK_INTERVAL=10
      - IMMUTABILITY_THRESHOLD=30
      - REDIS_SEGMENT_TTL=3600
      - REDIS_CLEANUP_THRESHOLD=86400
      - LOG_LEVEL=INFO
      - STORAGE_BACKEND=minio
      - MINIO_ENDPOINT=minio:9000
      - MINIO_ACCESS_KEY=vexa-access-key
      - MINIO_SECRET_KEY=vexa-secret-key
      - MINIO_BUCKET=vexa-recordings
      - MINIO_SECURE=false
    depends_on:
      redis:
        condition: service_started
      postgres:
        condition: service_healthy
    networks:
      - default
    restart: unless-stopped

  tts-service:
    build:
      context: .
      dockerfile: services/tts-service/Dockerfile
    expose:
      - "8002"
    environment:
      - OPENAI_API_KEY=${OPENAI_API_KEY:-}
      - LOG_LEVEL=INFO
    networks:
      - default
    restart: unless-stopped

  transcription-service:
    build:
      context: .
      dockerfile: services/transcription-service/Dockerfile.cpu
    expose:
      - "80"
    networks:
      - default
    restart: unless-stopped

  whisperlive-remote:
    build:
      context: .
      dockerfile: services/WhisperLive/Dockerfile.cpu
    volumes:
      - wl-recordings:/var/lib/wl-recordings
    environment:
      - REDIS_STREAM_URL=redis://redis:6379/0/transcription_segments
      - TRANSCRIPTION_COLLECTOR_URL=redis://redis:6379/0/transcription_segments
      - REDIS_HOST=redis
      - REDIS_PORT=6379
      - REDIS_DB=0
      - REDIS_STREAM_NAME=transcription_segments
      - LANGUAGE_DETECTION_SEGMENTS=10
      - VAD_FILTER_THRESHOLD=0.1
      - MIN_AUDIO_S=2.0
      - WL_RECORDING_DIR=/var/lib/wl-recordings
      - WL_RECORDING_FLUSH_SECONDS=3
      - WL_RECORDING_FSYNC_SECONDS=10
      - WL_RECORDING_ROTATE_SECONDS=20
      - WL_RECORDING_ROTATE_BYTES=16777216
      - WL_RECORDING_SNAPSHOT_SECONDS=20
      - WL_RECORDING_UPLOAD_URL=http://bot-manager:8080/internal/recordings/upload
      - SAME_OUTPUT_THRESHOLD=3
      - DEVICE_TYPE=remote
      - REMOTE_TRANSCRIBER_URL=http://transcription-service:80/v1/audio/transcriptions
      - REMOTE_TRANSCRIBER_API_KEY=internal-key
      - CONSUL_ENABLE=false
    deploy:
      replicas: 1
    command: >-
      --port 9090
      --backend remote
      --min_audio_s 2.0
      --wl_recording_dir /var/lib/wl-recordings
      --wl_recording_flush_seconds 3
      --wl_recording_fsync_seconds 10
      --wl_recording_rotate_seconds 20
      --wl_recording_rotate_bytes 16777216
      --wl_recording_snapshot_seconds 20
      --same_output_threshold 3
    expose:
      - "9090"
      - "9091"
    healthcheck:
      test: ["CMD", "python", "-c", "import urllib.request; urllib.request.urlopen('http://localhost:9091/health')"]
      interval: 10s
      timeout: 5s
      retries: 5
      start_period: 15s
    depends_on:
      transcription-collector:
        condition: service_started
      transcription-service:
        condition: service_started
    networks:
      - default
    restart: unless-stopped

  # --- Auto Setup ---
  init-setup:
    image: curlimages/curl:latest
    volumes:
      - setup-data:/shared
    environment:
      - ADMIN_API_TOKEN=auto-generated-admin-token
      - MEET_URL=${MEET_URL:-}
      - BOT_DISPLAY_NAME=${BOT_DISPLAY_NAME:-AI Assistant}
    entrypoint: ["/bin/sh", "/shared/init-setup.sh"]
    depends_on:
      api-gateway:
        condition: service_healthy
    networks:
      - default

  # --- AI Assistant ---
  meet-bot:
    build:
      context: .
      dockerfile: Dockerfile
    ports:
      - "${WEB_UI_PORT:-8080}:8080"
    environment:
      - VEXA_API_BASE=http://api-gateway:8000
      - VEXA_WS_URL=ws://api-gateway:8000/ws
      - OPENAI_API_KEY=${OPENAI_API_KEY:-}
      - OPENAI_MODEL=${OPENAI_MODEL:-gpt-4o}
      - TRIGGER_PHRASE=${TRIGGER_PHRASE:-hey bot}
      - BOT_DISPLAY_NAME=${BOT_DISPLAY_NAME:-AI Assistant}
      - SUMMARY_INTERVAL=${SUMMARY_INTERVAL:-10m}
      - TTS_PROVIDER=${TTS_PROVIDER:-openai}
      - TTS_VOICE=${TTS_VOICE:-nova}
      - MEET_URL=${MEET_URL:-}
      - VEXA_HEALTH_URL=http://api-gateway:8000/health
    volumes:
      - setup-data:/shared
    depends_on:
      init-setup:
        condition: service_completed_successfully
    networks:
      - default
    restart: unless-stopped

volumes:
  postgres-data:
  redis-data:
  minio-data:
  wl-recordings:
  setup-data:
    driver: local
    driver_opts:
      type: none
      o: bind
      device: ${PWD}/data/shared
```

**Step 2: Create data/shared directory and copy init script there**

The init-setup container needs the script available. We bind-mount `./data/shared` as the `setup-data` volume so the script and generated config persist on disk.

```bash
mkdir -p data/shared
```

**Step 3: Verify compose syntax**

```bash
OPENAI_API_KEY=test MEET_URL=https://meet.google.com/abc-defg-hij docker compose config --services
```

Expected: list of all services without errors.

**Step 4: Commit**

```bash
git add docker-compose.yml data/
git commit -m "feat: unified docker-compose with all Vexa services"
```

---

### Task 3: Create init-setup.sh

**Files:**
- Create: `scripts/init-setup.sh`
- Create: `data/shared/init-setup.sh` (symlink or copy at build time)

**Step 1: Write the init setup script**

Create `scripts/init-setup.sh`:

```bash
#!/bin/sh
set -e

ADMIN_URL="http://admin-api:8001"
GATEWAY_URL="http://api-gateway:8000"
SHARED="/shared"
MARKER="$SHARED/.setup-done"
KEY_FILE="$SHARED/api-key"

echo "[init-setup] Starting..."

# --- Step 1: Create user + API key (first boot only) ---
if [ -f "$MARKER" ] && [ -f "$KEY_FILE" ]; then
  echo "[init-setup] Setup already done, skipping user/key creation."
else
  echo "[init-setup] First boot — creating user and API key..."

  # Create user
  USER_RESP=$(curl -sf -X POST "$ADMIN_URL/admin/users" \
    -H "X-Admin-API-Key: $ADMIN_API_TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"email":"bot@meet-assistant.local","name":"Meet Bot"}' 2>&1) || {
    # User might already exist from a partial previous run
    echo "[init-setup] User creation response: $USER_RESP"
    USER_RESP=$(curl -sf "$ADMIN_URL/admin/users/email/bot@meet-assistant.local" \
      -H "X-Admin-API-Key: $ADMIN_API_TOKEN" 2>&1) || {
      echo "[init-setup] ERROR: Cannot create or find user"
      exit 1
    }
  }

  USER_ID=$(echo "$USER_RESP" | sed -n 's/.*"id":\([0-9]*\).*/\1/p' | head -1)
  if [ -z "$USER_ID" ]; then
    echo "[init-setup] ERROR: Could not extract user ID from: $USER_RESP"
    exit 1
  fi
  echo "[init-setup] User ID: $USER_ID"

  # Create API token
  TOKEN_RESP=$(curl -sf -X POST "$ADMIN_URL/admin/users/$USER_ID/tokens" \
    -H "X-Admin-API-Key: $ADMIN_API_TOKEN" \
    -H "Content-Type: application/json")
  API_KEY=$(echo "$TOKEN_RESP" | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')

  if [ -z "$API_KEY" ]; then
    echo "[init-setup] ERROR: Could not extract token from: $TOKEN_RESP"
    exit 1
  fi

  echo "$API_KEY" > "$KEY_FILE"
  touch "$MARKER"
  echo "[init-setup] API key saved to $KEY_FILE"
fi

API_KEY=$(cat "$KEY_FILE")

# --- Step 2: Parse MEET_URL and launch bot ---
if [ -z "$MEET_URL" ]; then
  echo "[init-setup] No MEET_URL set — skipping bot launch. Set it via web UI at http://localhost:8080"
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
    echo "[init-setup] ERROR: Unrecognized meeting URL: $MEET_URL"
    exit 1
    ;;
esac

if [ -z "$MEETING_ID" ]; then
  echo "[init-setup] ERROR: Could not parse meeting ID from: $MEET_URL"
  exit 1
fi

echo "[init-setup] Platform: $PLATFORM, Meeting ID: $MEETING_ID"

# Launch bot
echo "[init-setup] Launching bot into meeting..."
LAUNCH_RESP=$(curl -sf -X POST "$GATEWAY_URL/bots" \
  -H "X-API-Key: $API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"platform\":\"$PLATFORM\",\"native_meeting_id\":\"$MEETING_ID\",\"bot_name\":\"$BOT_DISPLAY_NAME\"}" 2>&1) || true

echo "[init-setup] Launch response: $LAUNCH_RESP"
echo "[init-setup] Done! Web UI at http://localhost:8080"
```

**Step 2: Copy to data/shared for the bind mount**

```bash
cp scripts/init-setup.sh data/shared/init-setup.sh
chmod +x scripts/init-setup.sh data/shared/init-setup.sh
```

**Step 3: Commit**

```bash
git add scripts/init-setup.sh data/shared/init-setup.sh
git commit -m "feat: init-setup script for auto user/key/bot creation"
```

---

### Task 4: Update config.go — MEET_URL parsing + file-based API key

**Files:**
- Modify: `internal/config/config.go`
- Modify: `internal/config/config_test.go`

**Step 1: Write failing tests for URL parsing**

Add to `internal/config/config_test.go`:

```go
func TestParseMeetURL(t *testing.T) {
	tests := []struct {
		url        string
		platform   string
		meetingID  string
		wantErr    bool
	}{
		{"https://meet.google.com/abc-defg-hij", "google_meet", "abc-defg-hij", false},
		{"https://meet.google.com/abc-defg-hij?authuser=0", "google_meet", "abc-defg-hij", false},
		{"https://teams.microsoft.com/l/meetup-join/abc123", "msteams", "l/meetup-join/abc123", false},
		{"https://zoom.us/j/12345678", "zoom", "12345678", false},
		{"https://us05web.zoom.us/j/12345678?pwd=abc", "zoom", "12345678", false},
		{"https://example.com/meeting", "", "", true},
		{"", "", "", true},
	}
	for _, tt := range tests {
		platform, id, err := parseMeetURL(tt.url)
		if tt.wantErr {
			if err == nil {
				t.Errorf("parseMeetURL(%q) expected error", tt.url)
			}
			continue
		}
		if err != nil {
			t.Errorf("parseMeetURL(%q) unexpected error: %v", tt.url, err)
			continue
		}
		if platform != tt.platform || id != tt.meetingID {
			t.Errorf("parseMeetURL(%q) = (%q, %q), want (%q, %q)",
				tt.url, platform, id, tt.platform, tt.meetingID)
		}
	}
}

func TestLoadWithMeetURL(t *testing.T) {
	t.Setenv("OPENAI_API_KEY", "test-key")
	t.Setenv("MEET_URL", "https://meet.google.com/abc-defg-hij")
	t.Setenv("VEXA_API_KEY", "test-vexa-key")

	cfg, err := Load()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if cfg.Platform != "google_meet" {
		t.Errorf("expected google_meet, got %s", cfg.Platform)
	}
	if cfg.NativeMeetingID != "abc-defg-hij" {
		t.Errorf("expected abc-defg-hij, got %s", cfg.NativeMeetingID)
	}
}

func TestLoadAPIKeyFromFile(t *testing.T) {
	t.Setenv("OPENAI_API_KEY", "test-key")
	t.Setenv("PLATFORM", "google_meet")
	t.Setenv("NATIVE_MEETING_ID", "test-id")

	// Create temp file with API key
	dir := t.TempDir()
	keyFile := dir + "/api-key"
	os.WriteFile(keyFile, []byte("file-based-key\n"), 0644)

	cfg, err := LoadWithKeyFile(keyFile)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if cfg.VexaAPIKey != "file-based-key" {
		t.Errorf("expected file-based-key, got %s", cfg.VexaAPIKey)
	}
}
```

**Step 2: Run tests to verify they fail**

```bash
cd /Users/kosmodev/Documents/pet_project/ai_google_meet_asistant
go test ./internal/config/ -v -run "TestParseMeetURL|TestLoadWithMeetURL|TestLoadAPIKeyFromFile"
```

Expected: FAIL — `parseMeetURL` and `LoadWithKeyFile` undefined.

**Step 3: Implement config changes**

Replace `internal/config/config.go` with:

```go
package config

import (
	"encoding/json"
	"fmt"
	"net/url"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/joho/godotenv"
)

type Config struct {
	// Vexa connection
	VexaAPIBase string
	VexaWSURL   string
	VexaAPIKey  string

	// Meeting identity
	Platform        string
	NativeMeetingID string
	MeetURL         string

	// OpenAI
	OpenAIAPIKey string
	OpenAIModel  string

	// Bot behavior
	TriggerPhrase   string
	BotDisplayName  string
	SummaryInterval time.Duration
	SystemPrompt    string

	// TTS
	TTSProvider string
	TTSVoice    string

	// Web UI
	WebUIPort string

	// Vexa health (for status checks)
	VexaHealthURL string

	// Config file path (for persistence)
	ConfigFile string

	mu sync.RWMutex
}

// HotReloadable returns fields that can change without restart.
type HotReloadable struct {
	TriggerPhrase  string `json:"trigger_phrase"`
	BotDisplayName string `json:"bot_display_name"`
	TTSVoice       string `json:"tts_voice"`
	OpenAIModel    string `json:"openai_model"`
	SystemPrompt   string `json:"system_prompt"`
}

// ConfigJSON is the on-disk config format.
type ConfigJSON struct {
	MeetURL        string `json:"meet_url,omitempty"`
	OpenAIAPIKey   string `json:"openai_api_key,omitempty"`
	OpenAIModel    string `json:"openai_model,omitempty"`
	TriggerPhrase  string `json:"trigger_phrase,omitempty"`
	BotDisplayName string `json:"bot_display_name,omitempty"`
	TTSVoice       string `json:"tts_voice,omitempty"`
	TTSProvider    string `json:"tts_provider,omitempty"`
	SystemPrompt   string `json:"system_prompt,omitempty"`
}

const DefaultKeyFile = "/shared/api-key"
const DefaultConfigFile = "/shared/config.json"

func Load() (*Config, error) {
	return LoadWithKeyFile(DefaultKeyFile)
}

func LoadWithKeyFile(keyFile string) (*Config, error) {
	_ = godotenv.Load()

	// Try config.json first
	configFile := getEnv("CONFIG_FILE", DefaultConfigFile)
	var fileConfig ConfigJSON
	if data, err := os.ReadFile(configFile); err == nil {
		_ = json.Unmarshal(data, &fileConfig)
	}

	// OpenAI key: env > config.json
	openAIKey := getEnv("OPENAI_API_KEY", fileConfig.OpenAIAPIKey)
	if openAIKey == "" {
		return nil, fmt.Errorf("required: OPENAI_API_KEY (env or config.json)")
	}

	// Vexa API key: env > file
	vexaKey := os.Getenv("VEXA_API_KEY")
	if vexaKey == "" {
		if data, err := os.ReadFile(keyFile); err == nil {
			vexaKey = strings.TrimSpace(string(data))
		}
	}
	if vexaKey == "" {
		return nil, fmt.Errorf("required: VEXA_API_KEY (env or %s file)", keyFile)
	}

	// Meeting: MEET_URL > config.json > PLATFORM+NATIVE_MEETING_ID
	meetURL := getEnv("MEET_URL", fileConfig.MeetURL)
	platform := os.Getenv("PLATFORM")
	meetingID := os.Getenv("NATIVE_MEETING_ID")

	if meetURL != "" && platform == "" {
		var err error
		platform, meetingID, err = parseMeetURL(meetURL)
		if err != nil {
			return nil, fmt.Errorf("invalid MEET_URL: %w", err)
		}
	}

	// Platform+MeetingID not strictly required at boot — can be set via UI later
	// But if neither is provided, log a warning (bot won't subscribe until set)

	return &Config{
		VexaAPIBase:     getEnv("VEXA_API_BASE", "http://api-gateway:8000"),
		VexaWSURL:       getEnv("VEXA_WS_URL", "ws://api-gateway:8000/ws"),
		VexaAPIKey:      vexaKey,
		Platform:        platform,
		NativeMeetingID: meetingID,
		MeetURL:         meetURL,
		OpenAIAPIKey:    openAIKey,
		OpenAIModel:     firstNonEmpty(fileConfig.OpenAIModel, getEnv("OPENAI_MODEL", "gpt-4o")),
		TriggerPhrase:   firstNonEmpty(fileConfig.TriggerPhrase, getEnv("TRIGGER_PHRASE", "hey bot")),
		BotDisplayName:  firstNonEmpty(fileConfig.BotDisplayName, getEnv("BOT_DISPLAY_NAME", "AI Assistant")),
		SummaryInterval: parseDuration(getEnv("SUMMARY_INTERVAL", "10m")),
		SystemPrompt:    fileConfig.SystemPrompt,
		TTSProvider:     firstNonEmpty(fileConfig.TTSProvider, getEnv("TTS_PROVIDER", "openai")),
		TTSVoice:        firstNonEmpty(fileConfig.TTSVoice, getEnv("TTS_VOICE", "nova")),
		WebUIPort:       getEnv("WEB_UI_PORT", "8080"),
		VexaHealthURL:   getEnv("VEXA_HEALTH_URL", "http://api-gateway:8000/health"),
		ConfigFile:      configFile,
	}, nil
}

// GetHotReloadable returns current hot-reloadable values (thread-safe).
func (c *Config) GetHotReloadable() HotReloadable {
	c.mu.RLock()
	defer c.mu.RUnlock()
	return HotReloadable{
		TriggerPhrase:  c.TriggerPhrase,
		BotDisplayName: c.BotDisplayName,
		TTSVoice:       c.TTSVoice,
		OpenAIModel:    c.OpenAIModel,
		SystemPrompt:   c.SystemPrompt,
	}
}

// ApplyHotReload updates hot-reloadable fields (thread-safe).
func (c *Config) ApplyHotReload(h HotReloadable) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if h.TriggerPhrase != "" {
		c.TriggerPhrase = h.TriggerPhrase
	}
	if h.BotDisplayName != "" {
		c.BotDisplayName = h.BotDisplayName
	}
	if h.TTSVoice != "" {
		c.TTSVoice = h.TTSVoice
	}
	if h.OpenAIModel != "" {
		c.OpenAIModel = h.OpenAIModel
	}
	c.SystemPrompt = h.SystemPrompt
}

// SaveConfigJSON writes current config to the config file.
func (c *Config) SaveConfigJSON() error {
	c.mu.RLock()
	cj := ConfigJSON{
		MeetURL:        c.MeetURL,
		OpenAIAPIKey:   c.OpenAIAPIKey,
		OpenAIModel:    c.OpenAIModel,
		TriggerPhrase:  c.TriggerPhrase,
		BotDisplayName: c.BotDisplayName,
		TTSVoice:       c.TTSVoice,
		TTSProvider:    c.TTSProvider,
		SystemPrompt:   c.SystemPrompt,
	}
	c.mu.RUnlock()

	data, err := json.MarshalIndent(cj, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(c.ConfigFile, data, 0644)
}

// ToConfigJSON exports current config as ConfigJSON.
func (c *Config) ToConfigJSON() ConfigJSON {
	c.mu.RLock()
	defer c.mu.RUnlock()
	return ConfigJSON{
		MeetURL:        c.MeetURL,
		OpenAIModel:    c.OpenAIModel,
		TriggerPhrase:  c.TriggerPhrase,
		BotDisplayName: c.BotDisplayName,
		TTSVoice:       c.TTSVoice,
		TTSProvider:    c.TTSProvider,
		SystemPrompt:   c.SystemPrompt,
	}
}

func parseMeetURL(rawURL string) (platform, meetingID string, err error) {
	if rawURL == "" {
		return "", "", fmt.Errorf("empty URL")
	}
	u, err := url.Parse(rawURL)
	if err != nil {
		return "", "", err
	}

	host := strings.ToLower(u.Host)
	switch {
	case strings.Contains(host, "meet.google.com"):
		parts := strings.Split(strings.Trim(u.Path, "/"), "/")
		if len(parts) == 0 || parts[0] == "" {
			return "", "", fmt.Errorf("no meeting code in Google Meet URL")
		}
		return "google_meet", parts[0], nil

	case strings.Contains(host, "teams.microsoft.com") || strings.Contains(host, "teams.live.com"):
		path := strings.TrimPrefix(u.Path, "/")
		if path == "" {
			return "", "", fmt.Errorf("no meeting path in Teams URL")
		}
		return "msteams", path, nil

	case strings.Contains(host, "zoom.us"):
		parts := strings.Split(u.Path, "/j/")
		if len(parts) < 2 || parts[1] == "" {
			return "", "", fmt.Errorf("no meeting ID in Zoom URL")
		}
		// Strip query params from meeting ID
		zoomID := strings.Split(parts[1], "?")[0]
		return "zoom", zoomID, nil

	default:
		return "", "", fmt.Errorf("unrecognized meeting platform: %s", host)
	}
}

func requireEnv(key string) (string, error) {
	v := os.Getenv(key)
	if v == "" {
		return "", fmt.Errorf("required env var missing: %s", key)
	}
	return v, nil
}

func getEnv(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}

func firstNonEmpty(values ...string) string {
	for _, v := range values {
		if v != "" {
			return v
		}
	}
	return ""
}

func parseDuration(s string) time.Duration {
	d, _ := time.ParseDuration(s)
	if d == 0 {
		return 10 * time.Minute
	}
	return d
}
```

**Step 4: Run tests to verify they pass**

```bash
go test ./internal/config/ -v
```

Expected: all tests PASS (update old tests that break due to signature change — `TestLoad` now needs only `OPENAI_API_KEY` + `MEET_URL` or the old combo with a key file).

**Step 5: Fix any broken tests**

The old `TestLoad` and `TestLoadMissingRequired` tests need updating. `TestLoad` should use `LoadWithKeyFile` with a temp key file, or set `VEXA_API_KEY` env. Update as needed to match new signatures.

**Step 6: Commit**

```bash
git add internal/config/
git commit -m "feat: config.go with MEET_URL parsing, file-based API key, config.json"
```

---

### Task 5: Add hot-reload to LLM Agent

**Files:**
- Modify: `internal/llm/openai.go`
- Modify: `internal/llm/openai_test.go`

**Step 1: Write failing test for UpdateSettings**

Add to `internal/llm/openai_test.go`:

```go
func TestUpdateSettings(t *testing.T) {
	a := New("fake-key", "gpt-4o", "Bot", "hey bot", "", zap.NewNop())

	a.UpdateSettings("new trigger", "New Bot", "custom prompt")

	// Trigger should be updated
	_, ok := a.ShouldRespond("new trigger what time")
	if !ok {
		t.Error("expected trigger 'new trigger' to match")
	}

	// Old trigger should not match
	_, ok = a.ShouldRespond("hey bot what time")
	if ok {
		t.Error("expected old trigger 'hey bot' to NOT match")
	}
}
```

**Step 2: Run test to verify it fails**

```bash
go test ./internal/llm/ -v -run TestUpdateSettings
```

Expected: FAIL — `New` has wrong number of args, `UpdateSettings` undefined.

**Step 3: Update openai.go**

Add `customSystemPrompt` field and `UpdateSettings` method. Modify `New` to accept optional system prompt:

```go
func New(apiKey, model, botName, triggerWord, customSystemPrompt string, log *zap.Logger) *Agent {
	systemMsg := customSystemPrompt
	if systemMsg == "" {
		systemMsg = "You are " + botName + ", an AI meeting assistant in a Google Meet call.\n" +
			"Respond ONLY when directly addressed with \"" + triggerWord + "\".\n" +
			"Keep responses concise (1-3 sentences). Respond in the same language as the question."
	}

	return &Agent{
		client:      openai.NewClient(apiKey),
		model:       model,
		systemMsg:   systemMsg,
		triggerWord: strings.ToLower(triggerWord),
		history: []openai.ChatCompletionMessage{
			{Role: openai.ChatMessageRoleSystem, Content: systemMsg},
		},
		log: log,
	}
}

// UpdateSettings hot-reloads trigger word, bot name, and system prompt.
func (a *Agent) UpdateSettings(triggerWord, botName, customSystemPrompt string) {
	a.mu.Lock()
	defer a.mu.Unlock()

	if triggerWord != "" {
		a.triggerWord = strings.ToLower(triggerWord)
	}

	if customSystemPrompt != "" {
		a.systemMsg = customSystemPrompt
	} else if botName != "" {
		a.systemMsg = "You are " + botName + ", an AI meeting assistant in a Google Meet call.\n" +
			"Respond ONLY when directly addressed with \"" + a.triggerWord + "\".\n" +
			"Keep responses concise (1-3 sentences). Respond in the same language as the question."
	}

	// Update system message in history
	if len(a.history) > 0 {
		a.history[0] = openai.ChatCompletionMessage{
			Role: openai.ChatMessageRoleSystem, Content: a.systemMsg,
		}
	}
}
```

**Step 4: Fix existing tests** — update all `New(...)` calls to add the 6th `""` argument for customSystemPrompt.

**Step 5: Run all tests**

```bash
go test ./internal/llm/ -v
```

Expected: all PASS.

**Step 6: Update bot.go** — change `llm.New` call to pass `cfg.SystemPrompt`:

In `internal/bot/bot.go`, line 34:
```go
agent := llm.New(cfg.OpenAIAPIKey, cfg.OpenAIModel,
    cfg.BotDisplayName, cfg.TriggerPhrase, cfg.SystemPrompt, log)
```

**Step 7: Commit**

```bash
git add internal/llm/ internal/bot/bot.go
git commit -m "feat: hot-reloadable LLM settings (trigger, name, system prompt)"
```

---

### Task 6: Create Web UI — server.go

**Files:**
- Create: `internal/web/server.go`

**Step 1: Write the HTTP server + API handlers**

Create `internal/web/server.go`:

```go
package web

import (
	"context"
	"embed"
	"encoding/json"
	"io"
	"net/http"
	"sync"
	"time"

	"github.com/gorilla/websocket"
	"go.uber.org/zap"

	"meet-bot/internal/config"
	"meet-bot/internal/llm"
	"meet-bot/internal/vexa"
)

//go:embed index.html
var staticFS embed.FS

var upgrader = websocket.Upgrader{
	CheckOrigin: func(r *http.Request) bool { return true },
}

type Server struct {
	cfg    *config.Config
	agent  *llm.Agent
	vexa   *vexa.Client
	log    *zap.Logger

	// Transcript broadcast
	mu          sync.RWMutex
	subscribers map[chan string]struct{}
}

func NewServer(cfg *config.Config, agent *llm.Agent, vexaClient *vexa.Client, log *zap.Logger) *Server {
	return &Server{
		cfg:         cfg,
		agent:       agent,
		vexa:        vexaClient,
		log:         log,
		subscribers: make(map[chan string]struct{}),
	}
}

// BroadcastTranscript sends a transcript line to all connected WebSocket clients.
func (s *Server) BroadcastTranscript(text string) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	for ch := range s.subscribers {
		select {
		case ch <- text:
		default: // skip slow clients
		}
	}
}

func (s *Server) Start(ctx context.Context) error {
	mux := http.NewServeMux()

	// Serve index.html
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		data, _ := staticFS.ReadFile("index.html")
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		w.Write(data)
	})

	// API endpoints
	mux.HandleFunc("/api/config", s.handleConfig)
	mux.HandleFunc("/api/status", s.handleStatus)
	mux.HandleFunc("/api/launch", s.handleLaunch)
	mux.HandleFunc("/api/stop", s.handleStop)
	mux.HandleFunc("/api/transcript", s.handleTranscriptWS)

	srv := &http.Server{
		Addr:    ":" + s.cfg.WebUIPort,
		Handler: mux,
	}

	go func() {
		<-ctx.Done()
		shutCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		srv.Shutdown(shutCtx)
	}()

	s.log.Info("web UI started", zap.String("port", s.cfg.WebUIPort))
	if err := srv.ListenAndServe(); err != http.ErrServerClosed {
		return err
	}
	return nil
}

func (s *Server) handleConfig(w http.ResponseWriter, r *http.Request) {
	switch r.Method {
	case http.MethodGet:
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(s.cfg.ToConfigJSON())

	case http.MethodPost:
		var cj config.ConfigJSON
		body, _ := io.ReadAll(r.Body)
		if err := json.Unmarshal(body, &cj); err != nil {
			http.Error(w, err.Error(), http.StatusBadRequest)
			return
		}

		// Apply hot-reloadable settings
		s.cfg.ApplyHotReload(config.HotReloadable{
			TriggerPhrase:  cj.TriggerPhrase,
			BotDisplayName: cj.BotDisplayName,
			TTSVoice:       cj.TTSVoice,
			OpenAIModel:    cj.OpenAIModel,
			SystemPrompt:   cj.SystemPrompt,
		})

		// Update agent
		s.agent.UpdateSettings(cj.TriggerPhrase, cj.BotDisplayName, cj.SystemPrompt)

		// Update meet URL if changed (needs restart indicator)
		needsRestart := false
		if cj.MeetURL != "" {
			s.cfg.MeetURL = cj.MeetURL
			needsRestart = true
		}
		if cj.OpenAIAPIKey != "" && cj.OpenAIAPIKey != s.cfg.OpenAIAPIKey {
			needsRestart = true
		}

		// Save to disk
		if err := s.cfg.SaveConfigJSON(); err != nil {
			s.log.Error("failed to save config", zap.Error(err))
		}

		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(map[string]any{
			"saved":         true,
			"needs_restart": needsRestart,
		})

	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

func (s *Server) handleStatus(w http.ResponseWriter, r *http.Request) {
	// Check Vexa health
	vexaHealthy := false
	if s.cfg.VexaHealthURL != "" {
		ctx, cancel := context.WithTimeout(r.Context(), 2*time.Second)
		defer cancel()
		req, _ := http.NewRequestWithContext(ctx, "GET", s.cfg.VexaHealthURL, nil)
		if resp, err := http.DefaultClient.Do(req); err == nil {
			vexaHealthy = resp.StatusCode == 200
			resp.Body.Close()
		}
	}

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]any{
		"vexa_healthy": vexaHealthy,
		"meet_url":     s.cfg.MeetURL,
		"platform":     s.cfg.Platform,
		"meeting_id":   s.cfg.NativeMeetingID,
		"trigger":      s.cfg.TriggerPhrase,
		"bot_name":     s.cfg.BotDisplayName,
	})
}

func (s *Server) handleLaunch(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}

	// Parse URL from request body or use current config
	var req struct {
		MeetURL string `json:"meet_url"`
	}
	if body, _ := io.ReadAll(r.Body); len(body) > 0 {
		json.Unmarshal(body, &req)
	}

	meetURL := req.MeetURL
	if meetURL == "" {
		meetURL = s.cfg.MeetURL
	}
	if meetURL == "" {
		http.Error(w, "no meeting URL configured", http.StatusBadRequest)
		return
	}

	// Forward to Vexa API
	platform := s.cfg.Platform
	meetingID := s.cfg.NativeMeetingID
	botName := s.cfg.BotDisplayName

	launchURL := s.cfg.VexaAPIBase + "/bots"
	body, _ := json.Marshal(map[string]string{
		"platform":          platform,
		"native_meeting_id": meetingID,
		"bot_name":          botName,
	})

	launchReq, _ := http.NewRequestWithContext(r.Context(), "POST", launchURL, io.NopCloser(
		strings.NewReader(string(body)),
	))
	launchReq.Header.Set("X-API-Key", s.cfg.VexaAPIKey)
	launchReq.Header.Set("Content-Type", "application/json")

	resp, err := http.DefaultClient.Do(launchReq)
	if err != nil {
		http.Error(w, err.Error(), http.StatusBadGateway)
		return
	}
	defer resp.Body.Close()

	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(resp.StatusCode)
	io.Copy(w, resp.Body)
}

func (s *Server) handleStop(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}

	stopURL := s.cfg.VexaAPIBase + "/bots/" + s.cfg.Platform + "/" + s.cfg.NativeMeetingID
	stopReq, _ := http.NewRequestWithContext(r.Context(), "DELETE", stopURL, nil)
	stopReq.Header.Set("X-API-Key", s.cfg.VexaAPIKey)

	resp, err := http.DefaultClient.Do(stopReq)
	if err != nil {
		http.Error(w, err.Error(), http.StatusBadGateway)
		return
	}
	defer resp.Body.Close()

	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(resp.StatusCode)
	io.Copy(w, resp.Body)
}

func (s *Server) handleTranscriptWS(w http.ResponseWriter, r *http.Request) {
	conn, err := upgrader.Upgrade(w, r, nil)
	if err != nil {
		return
	}
	defer conn.Close()

	ch := make(chan string, 50)
	s.mu.Lock()
	s.subscribers[ch] = struct{}{}
	s.mu.Unlock()

	defer func() {
		s.mu.Lock()
		delete(s.subscribers, ch)
		s.mu.Unlock()
	}()

	// Read pump (just drain, we only send)
	go func() {
		for {
			if _, _, err := conn.ReadMessage(); err != nil {
				close(ch)
				return
			}
		}
	}()

	for msg := range ch {
		if err := conn.WriteJSON(map[string]string{"text": msg}); err != nil {
			return
		}
	}
}
```

Note: `server.go` needs `import "strings"` added to the imports for `strings.NewReader` in `handleLaunch`.

**Step 2: Commit**

```bash
git add internal/web/server.go
git commit -m "feat: web UI HTTP server with config, status, launch, transcript APIs"
```

---

### Task 7: Create Web UI — index.html

**Files:**
- Create: `internal/web/index.html`

**Step 1: Write the single-page dashboard**

Create `internal/web/index.html` — a self-contained HTML file with inline CSS and JS. Features:
- Settings panel (left): meeting URL, OpenAI key, trigger, bot name, TTS voice dropdown, model dropdown
- System prompt panel (right): textarea
- Bot controls: Launch Bot / Stop Bot buttons
- Status indicators: bot connection, Vexa health
- Live transcript feed: WebSocket-powered scrolling log
- Responsive layout, dark theme, no external dependencies

The file is ~250 lines of HTML/CSS/JS. It:
- Fetches `/api/config` on load to populate fields
- POSTs to `/api/config` on "Save & Reload"
- POSTs to `/api/launch` / `/api/stop` for bot control
- Polls `/api/status` every 5s for health indicators
- Connects to `ws://host/api/transcript` for live transcript

```html
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>AI Meet Assistant</title>
<style>
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; background: #1a1a2e; color: #e0e0e0; min-height: 100vh; }
  .header { background: #16213e; padding: 16px 24px; display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid #0f3460; }
  .header h1 { font-size: 20px; color: #e94560; }
  .status-dot { width: 10px; height: 10px; border-radius: 50%; display: inline-block; margin-right: 6px; }
  .status-dot.green { background: #4ecca3; }
  .status-dot.red { background: #e94560; }
  .status-dot.yellow { background: #f0a500; }
  .main { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; padding: 16px; max-width: 1200px; margin: 0 auto; }
  .panel { background: #16213e; border-radius: 8px; padding: 20px; border: 1px solid #0f3460; }
  .panel h2 { font-size: 14px; text-transform: uppercase; color: #888; margin-bottom: 16px; letter-spacing: 1px; }
  .field { margin-bottom: 14px; }
  .field label { display: block; font-size: 13px; color: #aaa; margin-bottom: 4px; }
  .field input, .field select, .field textarea { width: 100%; padding: 8px 12px; background: #1a1a2e; border: 1px solid #0f3460; border-radius: 4px; color: #e0e0e0; font-size: 14px; }
  .field textarea { min-height: 200px; resize: vertical; font-family: monospace; }
  .field input:focus, .field select:focus, .field textarea:focus { outline: none; border-color: #e94560; }
  .btn { padding: 8px 20px; border: none; border-radius: 4px; cursor: pointer; font-size: 14px; font-weight: 600; }
  .btn-primary { background: #e94560; color: white; }
  .btn-primary:hover { background: #c73e54; }
  .btn-danger { background: #444; color: #e94560; border: 1px solid #e94560; }
  .btn-danger:hover { background: #e94560; color: white; }
  .btn-success { background: #4ecca3; color: #1a1a2e; }
  .btn-success:hover { background: #3db891; }
  .controls { display: flex; gap: 10px; margin-top: 16px; align-items: center; }
  .status-bar { display: flex; gap: 16px; margin-top: 12px; font-size: 13px; }
  .transcript-panel { grid-column: 1 / -1; }
  .transcript-feed { background: #0d1117; border-radius: 4px; padding: 12px; height: 300px; overflow-y: auto; font-family: monospace; font-size: 13px; line-height: 1.6; }
  .transcript-feed .line { padding: 2px 0; }
  .transcript-feed .line.bot { color: #4ecca3; }
  .transcript-feed .line.user { color: #e0e0e0; }
  .restart-badge { background: #f0a500; color: #1a1a2e; padding: 2px 8px; border-radius: 3px; font-size: 11px; font-weight: 600; margin-left: 8px; }
  .toast { position: fixed; bottom: 20px; right: 20px; background: #4ecca3; color: #1a1a2e; padding: 10px 20px; border-radius: 6px; font-weight: 600; display: none; z-index: 100; }
</style>
</head>
<body>
<div class="header">
  <h1>AI Meet Assistant</h1>
  <div>
    <span class="status-dot" id="vexa-dot"></span>
    <span id="vexa-status">Checking...</span>
  </div>
</div>

<div class="main">
  <div class="panel">
    <h2>Settings</h2>
    <div class="field">
      <label>Meeting URL</label>
      <input type="text" id="meet-url" placeholder="https://meet.google.com/abc-defg-hij">
    </div>
    <div class="field">
      <label>Trigger Phrase</label>
      <input type="text" id="trigger" placeholder="hey bot">
    </div>
    <div class="field">
      <label>Bot Display Name</label>
      <input type="text" id="bot-name" placeholder="AI Assistant">
    </div>
    <div class="field">
      <label>TTS Voice</label>
      <select id="tts-voice">
        <option value="nova">Nova</option>
        <option value="alloy">Alloy</option>
        <option value="echo">Echo</option>
        <option value="fable">Fable</option>
        <option value="onyx">Onyx</option>
        <option value="shimmer">Shimmer</option>
      </select>
    </div>
    <div class="field">
      <label>OpenAI Model</label>
      <select id="model">
        <option value="gpt-4o">GPT-4o</option>
        <option value="gpt-4o-mini">GPT-4o Mini</option>
        <option value="gpt-4-turbo">GPT-4 Turbo</option>
      </select>
    </div>
    <div class="controls">
      <button class="btn btn-primary" onclick="saveConfig()">Save &amp; Reload</button>
      <button class="btn btn-success" onclick="launchBot()">Launch Bot</button>
      <button class="btn btn-danger" onclick="stopBot()">Stop Bot</button>
    </div>
    <div class="status-bar">
      <div><span class="status-dot" id="bot-dot"></span> Bot: <span id="bot-status">unknown</span></div>
    </div>
  </div>

  <div class="panel">
    <h2>System Prompt</h2>
    <div class="field">
      <textarea id="system-prompt" placeholder="You are an AI meeting assistant...&#10;Respond concisely. Use the same language as the speaker."></textarea>
    </div>
    <button class="btn btn-primary" onclick="saveConfig()">Save Prompt</button>
  </div>

  <div class="panel transcript-panel">
    <h2>Live Transcript</h2>
    <div class="transcript-feed" id="transcript"></div>
  </div>
</div>

<div class="toast" id="toast"></div>

<script>
const $ = id => document.getElementById(id);

function toast(msg, ms = 2000) {
  const t = $('toast');
  t.textContent = msg;
  t.style.display = 'block';
  setTimeout(() => t.style.display = 'none', ms);
}

async function loadConfig() {
  try {
    const r = await fetch('/api/config');
    const c = await r.json();
    $('meet-url').value = c.meet_url || '';
    $('trigger').value = c.trigger_phrase || '';
    $('bot-name').value = c.bot_display_name || '';
    $('tts-voice').value = c.tts_voice || 'nova';
    $('model').value = c.openai_model || 'gpt-4o';
    $('system-prompt').value = c.system_prompt || '';
  } catch(e) { console.error('loadConfig', e); }
}

async function saveConfig() {
  try {
    const body = {
      meet_url: $('meet-url').value,
      trigger_phrase: $('trigger').value,
      bot_display_name: $('bot-name').value,
      tts_voice: $('tts-voice').value,
      openai_model: $('model').value,
      system_prompt: $('system-prompt').value,
    };
    const r = await fetch('/api/config', { method: 'POST', headers: {'Content-Type':'application/json'}, body: JSON.stringify(body) });
    const res = await r.json();
    toast(res.needs_restart ? 'Saved! Restart needed for some changes.' : 'Saved & applied!');
  } catch(e) { toast('Save failed: ' + e.message); }
}

async function launchBot() {
  try {
    const r = await fetch('/api/launch', { method: 'POST', headers: {'Content-Type':'application/json'}, body: JSON.stringify({meet_url: $('meet-url').value}) });
    if (r.ok) toast('Bot launched! Admit it in Google Meet.');
    else toast('Launch failed: ' + (await r.text()));
  } catch(e) { toast('Launch failed: ' + e.message); }
}

async function stopBot() {
  try {
    const r = await fetch('/api/stop', { method: 'POST' });
    if (r.ok) toast('Bot stopped.');
    else toast('Stop failed: ' + (await r.text()));
  } catch(e) { toast('Stop failed: ' + e.message); }
}

async function pollStatus() {
  try {
    const r = await fetch('/api/status');
    const s = await r.json();
    const vd = $('vexa-dot');
    const vs = $('vexa-status');
    if (s.vexa_healthy) { vd.className = 'status-dot green'; vs.textContent = 'Vexa: healthy'; }
    else { vd.className = 'status-dot red'; vs.textContent = 'Vexa: offline'; }
  } catch(e) {
    $('vexa-dot').className = 'status-dot red';
    $('vexa-status').textContent = 'Vexa: unreachable';
  }
}

function connectTranscript() {
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const ws = new WebSocket(proto + '//' + location.host + '/api/transcript');
  ws.onmessage = (e) => {
    const data = JSON.parse(e.data);
    const feed = $('transcript');
    const div = document.createElement('div');
    div.className = 'line user';
    const time = new Date().toLocaleTimeString('en-US', {hour12:false, hour:'2-digit', minute:'2-digit'});
    div.textContent = time + '  ' + data.text;
    feed.appendChild(div);
    feed.scrollTop = feed.scrollHeight;
  };
  ws.onclose = () => setTimeout(connectTranscript, 3000);
  ws.onerror = () => ws.close();
}

loadConfig();
pollStatus();
setInterval(pollStatus, 5000);
connectTranscript();
</script>
</body>
</html>
```

**Step 2: Commit**

```bash
git add internal/web/index.html
git commit -m "feat: web UI dashboard (settings, system prompt, transcript feed)"
```

---

### Task 8: Wire Web UI into main.go and bot.go

**Files:**
- Modify: `cmd/bot/main.go`
- Modify: `internal/bot/bot.go`

**Step 1: Update main.go to start web server alongside bot**

Replace `cmd/bot/main.go`:

```go
package main

import (
	"context"
	"os"
	"os/signal"
	"syscall"

	"go.uber.org/zap"

	"meet-bot/internal/bot"
	"meet-bot/internal/config"
	"meet-bot/internal/llm"
	"meet-bot/internal/vexa"
	"meet-bot/internal/web"
)

func main() {
	log, _ := zap.NewProduction()
	defer log.Sync() //nolint:errcheck

	cfg, err := config.Load()
	if err != nil {
		log.Fatal("config error", zap.Error(err))
	}

	vexaClient := vexa.NewClient(cfg.VexaAPIBase, cfg.VexaAPIKey, log)
	agent := llm.New(cfg.OpenAIAPIKey, cfg.OpenAIModel,
		cfg.BotDisplayName, cfg.TriggerPhrase, cfg.SystemPrompt, log)

	// Start web UI
	webSrv := web.NewServer(cfg, agent, vexaClient, log)

	ctx, stop := signal.NotifyContext(context.Background(),
		os.Interrupt, syscall.SIGTERM)
	defer stop()

	// Web UI in background
	go func() {
		if err := webSrv.Start(ctx); err != nil {
			log.Error("web UI error", zap.Error(err))
		}
	}()

	// Bot loop (only if meeting is configured)
	if cfg.Platform != "" && cfg.NativeMeetingID != "" {
		b := bot.NewWithDeps(cfg, vexaClient, agent, webSrv, log)
		if err := b.Run(ctx); err != nil {
			log.Fatal("bot error", zap.Error(err))
		}
	} else {
		log.Info("no meeting configured — web UI only mode. Set MEET_URL and launch via UI.")
		<-ctx.Done()
	}
}
```

**Step 2: Update bot.go — accept deps, broadcast transcripts to web UI**

Add `NewWithDeps` constructor and broadcast hook. Modify `internal/bot/bot.go`:

Add a `TranscriptBroadcaster` interface and accept it:

```go
// TranscriptBroadcaster sends transcript lines to web UI clients.
type TranscriptBroadcaster interface {
	BroadcastTranscript(text string)
}

func NewWithDeps(cfg *config.Config, vexaClient *vexa.Client, agent *llm.Agent, broadcaster TranscriptBroadcaster, log *zap.Logger) *Bot {
	wsClient := vexa.NewWSClient(
		cfg.VexaWSURL, cfg.VexaAPIKey,
		cfg.Platform, cfg.NativeMeetingID,
		log,
	)
	return &Bot{
		cfg:         cfg,
		vexa:        vexaClient,
		ws:          wsClient,
		agent:       agent,
		broadcaster: broadcaster,
		log:         log,
		segments:    make(map[string]vexa.Segment),
	}
}
```

Add `broadcaster TranscriptBroadcaster` field to the `Bot` struct. In `handleEvent`, after logging a transcript, broadcast it:

```go
b.log.Info("transcript", zap.String("speaker", seg.Speaker), zap.String("text", seg.Text))
if b.broadcaster != nil {
    b.broadcaster.BroadcastTranscript(formatted)
}
```

Keep the old `New` constructor working (pass `nil` for broadcaster) so existing tests don't break.

**Step 3: Verify build**

```bash
go build ./cmd/bot/
```

Expected: compiles without errors.

**Step 4: Commit**

```bash
git add cmd/bot/main.go internal/bot/bot.go
git commit -m "feat: wire web UI into main.go, broadcast transcripts to UI"
```

---

### Task 9: Update .env.example and Dockerfile

**Files:**
- Replace: `.env.example`
- Verify: `Dockerfile` (should work as-is since go:embed handles index.html)

**Step 1: Write minimal .env.example**

```bash
# Required
OPENAI_API_KEY=sk-...
MEET_URL=https://meet.google.com/abc-defg-hij

# Optional
# TRIGGER_PHRASE=hey bot
# BOT_DISPLAY_NAME=AI Assistant
# TTS_VOICE=nova
# OPENAI_MODEL=gpt-4o
# WEB_UI_PORT=8080
# API_GATEWAY_PORT=8056
```

**Step 2: Verify Dockerfile builds**

```bash
docker build -t meet-bot-test .
```

Expected: builds successfully (go:embed picks up `internal/web/index.html` automatically).

**Step 3: Commit**

```bash
git add .env.example Dockerfile
git commit -m "feat: simplified .env.example, verify Dockerfile"
```

---

### Task 10: Build vexa-bot image and test end-to-end

**Files:** None (integration test)

**Step 1: Build the vexa-bot image**

```bash
docker build -t vexa-bot:dev -f services/vexa-bot/Dockerfile services/vexa-bot/
```

**Step 2: Create .env from example**

```bash
cp .env.example .env
# Edit .env with real OPENAI_API_KEY and MEET_URL
```

**Step 3: Create shared data directory**

```bash
mkdir -p data/shared
cp scripts/init-setup.sh data/shared/init-setup.sh
```

**Step 4: Start everything**

```bash
docker compose up -d
```

**Step 5: Verify services start**

```bash
docker compose ps
# All services should be Up or completed (init-setup exits 0)
```

**Step 6: Open web UI**

Open `http://localhost:8080` — should see the dashboard with settings populated.

**Step 7: Test bot launch**

Click "Launch Bot" in the UI, admit it in Google Meet, speak "hey bot what is two plus two".

**Step 8: Commit**

```bash
git add -A
git commit -m "feat: unified AI Meet Assistant — one command setup"
```

---

### Task 11: Update README.md

**Files:**
- Modify: `README.md`

**Step 1: Rewrite README for the unified app**

Update the README to reflect the new one-command setup:

- Quick start: clone → set .env → `docker compose up` → open localhost:8080
- Architecture diagram updated to show all services in one compose
- Web UI screenshots/description
- Remove the old "add to Vexa's docker-compose" instructions
- Keep bilingual EN/UA format

**Step 2: Update CLAUDE.md**

Reflect new file paths, web UI, init-setup.

**Step 3: Commit**

```bash
git add README.md CLAUDE.md
git commit -m "docs: updated README and CLAUDE.md for unified app"
```
