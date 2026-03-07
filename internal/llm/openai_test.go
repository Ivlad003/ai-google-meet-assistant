package llm

import (
	"testing"

	"go.uber.org/zap"
)

func TestShouldRespondSkipsShortText(t *testing.T) {
	a := New("fake-key", "gpt-4o", "Bot", "hey bot", "", zap.NewNop())

	// Very short text should be skipped without calling LLM
	_, ok := a.ShouldRespond("hi")
	if ok {
		t.Error("expected short text to be rejected")
	}

	_, ok = a.ShouldRespond("    ")
	if ok {
		t.Error("expected whitespace to be rejected")
	}
}

func TestAddTranscript(t *testing.T) {
	a := New("fake-key", "gpt-4o", "Bot", "hey bot", "", zap.NewNop())
	for i := 0; i < 60; i++ {
		a.AddTranscript("line")
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	if len(a.transcript) != 50 {
		t.Errorf("transcript length = %d, want 50", len(a.transcript))
	}
}

func TestUpdateSettings(t *testing.T) {
	a := New("fake-key", "gpt-4o", "Bot", "hey bot", "", zap.NewNop())

	a.UpdateSettings("new trigger", "New Bot", "")

	settings := a.GetSettings()
	if settings.BotName != "New Bot" {
		t.Errorf("expected botName 'New Bot', got %q", settings.BotName)
	}
	if settings.TriggerWord != "new trigger" {
		t.Errorf("expected triggerWord 'new trigger', got %q", settings.TriggerWord)
	}
}

func TestUpdateSettingsCustomPrompt(t *testing.T) {
	a := New("fake-key", "gpt-4o", "Bot", "hey bot", "", zap.NewNop())

	a.UpdateSettings("", "", "You are a custom assistant.")

	settings := a.GetSettings()
	if settings.SystemPrompt != "You are a custom assistant." {
		t.Errorf("expected custom system prompt, got %q", settings.SystemPrompt)
	}

	// History[0] should also be updated
	a.mu.Lock()
	if a.history[0].Content != "You are a custom assistant." {
		t.Errorf("expected history[0] to have custom prompt, got %q", a.history[0].Content)
	}
	a.mu.Unlock()
}

func TestGetSettings(t *testing.T) {
	a := New("fake-key", "gpt-4o", "Bot", "hey bot", "", zap.NewNop())

	s := a.GetSettings()
	if s.TriggerWord != "hey bot" {
		t.Errorf("expected 'hey bot', got %q", s.TriggerWord)
	}
	if s.Model != "gpt-4o" {
		t.Errorf("expected 'gpt-4o', got %q", s.Model)
	}
	if s.BotName != "Bot" {
		t.Errorf("expected 'Bot', got %q", s.BotName)
	}
}

func TestNewWithCustomSystemPrompt(t *testing.T) {
	a := New("fake-key", "gpt-4o", "Bot", "hey bot", "Custom prompt here", zap.NewNop())

	s := a.GetSettings()
	if s.SystemPrompt != "Custom prompt here" {
		t.Errorf("expected 'Custom prompt here', got %q", s.SystemPrompt)
	}
}

func TestLastN(t *testing.T) {
	a := New("fake-key", "gpt-4o", "Bot", "hey bot", "", zap.NewNop())
	for i := 0; i < 10; i++ {
		a.transcript = append(a.transcript, "x")
	}
	if len(a.lastN(5)) != 5 {
		t.Errorf("lastN(5) = %d, want 5", len(a.lastN(5)))
	}
	if len(a.lastN(20)) != 10 {
		t.Errorf("lastN(20) = %d, want 10", len(a.lastN(20)))
	}
}
