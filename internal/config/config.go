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
	// But if neither is provided, bot won't subscribe until set

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

// GetSystemPrompt returns the current system prompt (thread-safe).
func (c *Config) GetSystemPrompt() string {
	c.mu.RLock()
	defer c.mu.RUnlock()
	return c.SystemPrompt
}

// SetSystemPrompt updates the system prompt (thread-safe).
func (c *Config) SetSystemPrompt(s string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.SystemPrompt = s
}

// UpdateFromJSON updates in-memory config from a JSON key-value map (thread-safe).
func (c *Config) UpdateFromJSON(data map[string]interface{}) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if v, ok := data["trigger_phrase"].(string); ok && v != "" {
		c.TriggerPhrase = v
	}
	if v, ok := data["bot_display_name"].(string); ok && v != "" {
		c.BotDisplayName = v
	}
	if v, ok := data["tts_voice"].(string); ok && v != "" {
		c.TTSVoice = v
	}
	if v, ok := data["tts_provider"].(string); ok && v != "" {
		c.TTSProvider = v
	}
	if v, ok := data["openai_model"].(string); ok && v != "" {
		c.OpenAIModel = v
	}
	if v, ok := data["system_prompt"].(string); ok {
		c.SystemPrompt = v
	}
	if v, ok := data["meet_url"].(string); ok && v != "" {
		c.MeetURL = v
		if platform, meetingID, err := parseMeetURL(v); err == nil {
			c.Platform = platform
			c.NativeMeetingID = meetingID
		}
	}
}

// SaveConfigJSON writes current config to the config file.
func (c *Config) SaveConfigJSON() error {
	return c.SaveConfigJSONTo(c.ConfigFile)
}

// SaveConfigJSONTo writes current config to the specified path.
func (c *Config) SaveConfigJSONTo(path string) error {
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
	return os.WriteFile(path, data, 0644)
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
