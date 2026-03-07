package llm

import (
	"context"
	"strings"
	"sync"
	"time"

	"github.com/sashabaranov/go-openai"
	"go.uber.org/zap"
)

// AgentSettings holds the current hot-reloadable settings.
type AgentSettings struct {
	TriggerWord  string `json:"triggerWord"`
	Model        string `json:"model"`
	BotName      string `json:"botName"`
	SystemPrompt string `json:"systemPrompt"`
}

type Agent struct {
	mu                 sync.Mutex
	client             *openai.Client
	model              string
	botName            string
	systemMsg          string
	customSystemPrompt string
	history            []openai.ChatCompletionMessage
	transcript         []string
	triggerWord        string
	log                *zap.Logger
}

func New(apiKey, model, botName, triggerWord, customSystemPrompt string, log *zap.Logger) *Agent {
	systemMsg := customSystemPrompt
	if systemMsg == "" {
		systemMsg = buildSystemMsg(botName, triggerWord)
	}

	return &Agent{
		client:             openai.NewClient(apiKey),
		model:              model,
		botName:            botName,
		systemMsg:          systemMsg,
		customSystemPrompt: customSystemPrompt,
		triggerWord:        strings.ToLower(triggerWord),
		history: []openai.ChatCompletionMessage{
			{Role: openai.ChatMessageRoleSystem, Content: systemMsg},
		},
		log: log,
	}
}

func buildSystemMsg(botName, triggerWord string) string {
	return "You are " + botName + ", an AI meeting assistant in a Google Meet call.\n" +
		"You respond when participants address you directly.\n" +
		"Keep responses concise (1-3 sentences).\n" +
		"IMPORTANT: Respond ONLY in English or Ukrainian. If the user speaks Ukrainian, respond in Ukrainian. Otherwise respond in English. NEVER respond in Russian."
}

// UpdateSettings hot-reloads trigger word, bot name, and system prompt.
func (a *Agent) UpdateSettings(triggerWord, botName, customSystemPrompt string) {
	a.mu.Lock()
	defer a.mu.Unlock()

	if triggerWord != "" {
		a.triggerWord = strings.ToLower(triggerWord)
	}

	if botName != "" {
		a.botName = botName
	}

	if customSystemPrompt != "" {
		a.customSystemPrompt = customSystemPrompt
		a.systemMsg = customSystemPrompt
	} else if botName != "" || triggerWord != "" {
		// Rebuild default system message with current values
		a.systemMsg = buildSystemMsg(a.botName, a.triggerWord)
	}

	// Update system message in history
	if len(a.history) > 0 {
		a.history[0] = openai.ChatCompletionMessage{
			Role: openai.ChatMessageRoleSystem, Content: a.systemMsg,
		}
	}
}

// GetSettings returns the current hot-reloadable settings.
func (a *Agent) GetSettings() AgentSettings {
	a.mu.Lock()
	defer a.mu.Unlock()
	return AgentSettings{
		TriggerWord:  a.triggerWord,
		Model:        a.model,
		BotName:      a.botName,
		SystemPrompt: a.systemMsg,
	}
}

func (a *Agent) AddTranscript(text string) {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.transcript = append(a.transcript, text)
	if len(a.transcript) > 50 {
		a.transcript = a.transcript[len(a.transcript)-50:]
	}
}

// ShouldRespond uses LLM to detect if someone is addressing the bot.
// Returns the extracted question and whether to respond.
func (a *Agent) ShouldRespond(text string) (string, bool) {
	// Skip very short text (noise)
	trimmed := strings.TrimSpace(text)
	if len(trimmed) < 5 {
		return "", false
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	a.mu.Lock()
	recentCtx := strings.Join(a.lastN(5), "\n")
	botName := a.botName
	trigger := a.triggerWord
	a.mu.Unlock()

	prompt := `You are an intent detector for a meeting bot named "` + botName + `".
The trigger phrase is "` + trigger + `" but speech recognition often misheard it (e.g. "hey what", "hi buddy", "high board", "hey boss", etc.).

Given the transcript line below, determine if the speaker is addressing the bot.
Consider:
- Any phrase that sounds like "` + trigger + `" (even badly transcribed)
- Directly mentioning the bot by name
- Asking a question clearly directed at an AI assistant

Recent meeting context:
` + recentCtx + `

New transcript line: "` + trimmed + `"

Respond with EXACTLY one of:
- "YES: <the question they're asking>" if they are addressing the bot
- "NO" if they are not addressing the bot

Examples:
- "Hey boss, what is 2 plus 2?" → YES: what is 2 plus 2?
- "High board, can you summarize?" → YES: can you summarize?
- "I think we should schedule a meeting" → NO
- "Bot, help me" → YES: help me`

	resp, err := a.client.CreateChatCompletion(ctx, openai.ChatCompletionRequest{
		Model: "gpt-4o-mini",
		Messages: []openai.ChatCompletionMessage{
			{Role: openai.ChatMessageRoleUser, Content: prompt},
		},
		Temperature: 0,
		MaxTokens:   60,
	})
	if err != nil {
		a.log.Debug("intent detection error", zap.Error(err))
		return "", false
	}

	answer := strings.TrimSpace(resp.Choices[0].Message.Content)
	a.log.Debug("intent detection", zap.String("text", trimmed), zap.String("result", answer))

	if strings.HasPrefix(answer, "YES:") {
		question := strings.TrimSpace(strings.TrimPrefix(answer, "YES:"))
		if question == "" {
			return trimmed, true
		}
		return question, true
	}
	if answer == "YES" {
		return trimmed, true
	}

	return "", false
}

func (a *Agent) Respond(ctx context.Context, question string) (string, error) {
	a.mu.Lock()
	recentCtx := strings.Join(a.lastN(10), "\n")
	a.mu.Unlock()

	userMsg := "[Recent meeting context]:\n" + recentCtx +
		"\n\n[Question to you]: " + question

	a.history = append(a.history, openai.ChatCompletionMessage{
		Role:    openai.ChatMessageRoleUser,
		Content: userMsg,
	})

	resp, err := a.client.CreateChatCompletion(ctx, openai.ChatCompletionRequest{
		Model:       a.model,
		Messages:    a.history,
		Temperature: 0.7,
		MaxTokens:   150,
	})
	if err != nil {
		return "", err
	}

	answer := strings.TrimSpace(resp.Choices[0].Message.Content)
	a.history = append(a.history, openai.ChatCompletionMessage{
		Role:    openai.ChatMessageRoleAssistant,
		Content: answer,
	})

	// Keep system message + last 20 exchanges
	if len(a.history) > 41 {
		a.history = append(a.history[:1], a.history[len(a.history)-40:]...)
	}

	return answer, nil
}

func (a *Agent) Summary(ctx context.Context) (string, error) {
	a.mu.Lock()
	transcriptCopy := make([]string, len(a.transcript))
	copy(transcriptCopy, a.transcript)
	a.mu.Unlock()

	prompt := "Provide a brief meeting summary (3-5 bullet points) based on:\n" +
		strings.Join(transcriptCopy, "\n")

	resp, err := a.client.CreateChatCompletion(ctx, openai.ChatCompletionRequest{
		Model: a.model,
		Messages: []openai.ChatCompletionMessage{
			{Role: openai.ChatMessageRoleSystem, Content: a.systemMsg},
			{Role: openai.ChatMessageRoleUser, Content: prompt},
		},
	})
	if err != nil {
		return "", err
	}

	return strings.TrimSpace(resp.Choices[0].Message.Content), nil
}

func (a *Agent) lastN(n int) []string {
	if len(a.transcript) <= n {
		return a.transcript
	}
	return a.transcript[len(a.transcript)-n:]
}
