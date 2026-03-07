package config

import (
	"os"
	"testing"
	"time"
)

func TestGetEnv(t *testing.T) {
	t.Setenv("TEST_KEY", "test_value")

	if v := getEnv("TEST_KEY", "fallback"); v != "test_value" {
		t.Errorf("expected test_value, got %s", v)
	}
	if v := getEnv("MISSING_KEY", "fallback"); v != "fallback" {
		t.Errorf("expected fallback, got %s", v)
	}
}

func TestParseDuration(t *testing.T) {
	if v := parseDuration("5m"); v != 5*time.Minute {
		t.Errorf("expected 5m, got %v", v)
	}
	if v := parseDuration(""); v != 10*time.Minute {
		t.Errorf("expected default 10m, got %v", v)
	}
}

func TestRequireEnvReturnsError(t *testing.T) {
	_, err := requireEnv("DEFINITELY_NOT_SET_VAR_XYZ")
	if err == nil {
		t.Error("expected error for missing required env var")
	}
}

func TestRequireEnvReturnsValue(t *testing.T) {
	t.Setenv("TEST_REQUIRE", "hello")
	v, err := requireEnv("TEST_REQUIRE")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if v != "hello" {
		t.Errorf("expected hello, got %s", v)
	}
}

func TestLoadMissingRequired(t *testing.T) {
	_, err := Load()
	if err == nil {
		t.Error("expected error when required env vars are missing")
	}
}

func TestLoad(t *testing.T) {
	t.Setenv("VEXA_API_KEY", "test-vexa-key")
	t.Setenv("OPENAI_API_KEY", "test-openai-key")
	t.Setenv("PLATFORM", "google_meet")
	t.Setenv("NATIVE_MEETING_ID", "abc-defg-hij")

	cfg, err := Load()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if cfg.VexaAPIKey != "test-vexa-key" {
		t.Errorf("expected test-vexa-key, got %s", cfg.VexaAPIKey)
	}
	if cfg.OpenAIModel != "gpt-4o" {
		t.Errorf("expected default gpt-4o, got %s", cfg.OpenAIModel)
	}
	if cfg.BotDisplayName != "AI Assistant" {
		t.Errorf("expected default AI Assistant, got %s", cfg.BotDisplayName)
	}
	if cfg.TTSVoice != "nova" {
		t.Errorf("expected default nova, got %s", cfg.TTSVoice)
	}
}

func TestParseMeetURL(t *testing.T) {
	tests := []struct {
		url       string
		platform  string
		meetingID string
		wantErr   bool
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

func TestFirstNonEmpty(t *testing.T) {
	if v := firstNonEmpty("", "", "c"); v != "c" {
		t.Errorf("expected c, got %s", v)
	}
	if v := firstNonEmpty("a", "b"); v != "a" {
		t.Errorf("expected a, got %s", v)
	}
	if v := firstNonEmpty("", ""); v != "" {
		t.Errorf("expected empty, got %s", v)
	}
}

func TestGetSetSystemPrompt(t *testing.T) {
	cfg := &Config{}
	cfg.SetSystemPrompt("test prompt")
	if got := cfg.GetSystemPrompt(); got != "test prompt" {
		t.Errorf("expected 'test prompt', got %q", got)
	}
}

func TestUpdateFromJSON(t *testing.T) {
	cfg := &Config{
		TriggerPhrase:  "old phrase",
		BotDisplayName: "old name",
	}
	cfg.UpdateFromJSON(map[string]interface{}{
		"trigger_phrase":  "new phrase",
		"bot_display_name": "new name",
		"meet_url":        "https://meet.google.com/xyz-abcd-efg",
	})
	if cfg.TriggerPhrase != "new phrase" {
		t.Errorf("expected 'new phrase', got %q", cfg.TriggerPhrase)
	}
	if cfg.BotDisplayName != "new name" {
		t.Errorf("expected 'new name', got %q", cfg.BotDisplayName)
	}
	if cfg.Platform != "google_meet" {
		t.Errorf("expected google_meet, got %s", cfg.Platform)
	}
	if cfg.NativeMeetingID != "xyz-abcd-efg" {
		t.Errorf("expected xyz-abcd-efg, got %s", cfg.NativeMeetingID)
	}
}

func TestSaveAndLoadConfigJSON(t *testing.T) {
	dir := t.TempDir()
	path := dir + "/config.json"

	cfg := &Config{
		MeetURL:        "https://meet.google.com/abc-defg-hij",
		OpenAIAPIKey:   "sk-test",
		OpenAIModel:    "gpt-4o",
		TriggerPhrase:  "hey bot",
		BotDisplayName: "Test Bot",
		TTSVoice:       "nova",
		TTSProvider:    "openai",
		SystemPrompt:   "You are helpful.",
		ConfigFile:     path,
	}

	if err := cfg.SaveConfigJSON(); err != nil {
		t.Fatalf("SaveConfigJSON error: %v", err)
	}

	// Verify file exists and can be read back
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("failed to read saved config: %v", err)
	}
	if len(data) == 0 {
		t.Error("saved config file is empty")
	}
}

func TestHotReload(t *testing.T) {
	cfg := &Config{
		TriggerPhrase:  "hey bot",
		BotDisplayName: "Bot",
		TTSVoice:       "nova",
		OpenAIModel:    "gpt-4o",
		SystemPrompt:   "",
	}

	cfg.ApplyHotReload(HotReloadable{
		TriggerPhrase: "yo bot",
		OpenAIModel:   "gpt-4o-mini",
		SystemPrompt:  "Be concise.",
	})

	hr := cfg.GetHotReloadable()
	if hr.TriggerPhrase != "yo bot" {
		t.Errorf("expected 'yo bot', got %q", hr.TriggerPhrase)
	}
	if hr.OpenAIModel != "gpt-4o-mini" {
		t.Errorf("expected gpt-4o-mini, got %q", hr.OpenAIModel)
	}
	if hr.SystemPrompt != "Be concise." {
		t.Errorf("expected 'Be concise.', got %q", hr.SystemPrompt)
	}
	// Empty fields should NOT overwrite
	if hr.BotDisplayName != "Bot" {
		t.Errorf("expected 'Bot', got %q", hr.BotDisplayName)
	}
	if hr.TTSVoice != "nova" {
		t.Errorf("expected 'nova', got %q", hr.TTSVoice)
	}
}
