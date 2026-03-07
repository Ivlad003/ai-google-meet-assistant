package bot

import (
	"context"
	"strings"
	"sync/atomic"
	"time"

	"go.uber.org/zap"

	"meet-bot/internal/config"
	"meet-bot/internal/llm"
	"meet-bot/internal/vexa"
)

type Bot struct {
	cfg   *config.Config
	vexa  *vexa.Client
	ws    *vexa.WSClient
	agent *llm.Agent
	log   *zap.Logger

	// transcript map keyed by absolute_start_time for dedup
	segments map[string]vexa.Segment

	// broadcast sends transcript lines to web UI (optional)
	broadcast func(string)
}

func New(cfg *config.Config, log *zap.Logger) *Bot {
	vexaClient := vexa.NewClient(cfg.VexaAPIBase, cfg.VexaAPIKey, log)
	wsClient := vexa.NewWSClient(
		cfg.VexaWSURL, cfg.VexaAPIKey,
		cfg.Platform, cfg.NativeMeetingID,
		log,
	)
	agent := llm.New(cfg.OpenAIAPIKey, cfg.OpenAIModel,
		cfg.BotDisplayName, cfg.TriggerPhrase, cfg.SystemPrompt, log)

	return &Bot{
		cfg:      cfg,
		vexa:     vexaClient,
		ws:       wsClient,
		agent:    agent,
		log:      log,
		segments: make(map[string]vexa.Segment),
	}
}

// Agent returns the LLM agent (for web UI integration).
func (b *Bot) Agent() *llm.Agent { return b.agent }

// VexaClient returns the Vexa REST client (for web UI integration).
func (b *Bot) VexaClient() *vexa.Client { return b.vexa }

// SetBroadcast sets a callback for broadcasting transcript lines to the web UI.
func (b *Bot) SetBroadcast(fn func(string)) { b.broadcast = fn }

func (b *Bot) Run(ctx context.Context) error {
	// 1. Wait for Vexa to be ready and connect
	events, err := b.connectWithRetry(ctx)
	if err != nil {
		return err
	}
	defer b.ws.Close()

	// 2. Main event loop with reconnect on subscribe errors
	var speaking atomic.Bool
	summaryTicker := time.NewTicker(b.cfg.SummaryInterval)
	defer summaryTicker.Stop()

	for {
		select {
		case <-ctx.Done():
			b.log.Info("shutting down...")
			b.generateFinalSummary()
			return nil

		case event, ok := <-events:
			if !ok {
				b.log.Info("WebSocket closed, reconnecting...")
				b.ws.Close()
				var err error
				events, err = b.connectWithRetry(ctx)
				if err != nil {
					return err
				}
				continue
			}

			// If subscribe failed (meeting not in Vexa yet), retry
			if event.Type == vexa.EventError && isRetryableError(event.Error) {
				b.log.Warn("subscribe error, meeting may not exist yet — reconnecting...", zap.String("error", event.Error))
				b.ws.Close()
				if waitErr := b.backoff(ctx, 1); waitErr != nil {
					return waitErr
				}
				var err error
				events, err = b.connectWithRetry(ctx)
				if err != nil {
					return err
				}
				continue
			}

			b.handleEvent(ctx, event, &speaking)

		case <-summaryTicker.C:
			go func() {
				summary, err := b.agent.Summary(ctx)
				if err != nil {
					b.log.Error("summary error", zap.Error(err))
					return
				}
				b.log.Info("MEETING SUMMARY\n" + summary)
			}()
		}
	}
}

func isRetryableError(errMsg string) bool {
	return errMsg == "invalid_subscribe_payload" ||
		errMsg == "authorization_call_failed" ||
		errMsg == "authorization_service_error"
}

func (b *Bot) connectWithRetry(ctx context.Context) (<-chan vexa.WSEvent, error) {
	for attempt := 1; ; attempt++ {
		// Bootstrap transcript from REST
		b.log.Info("bootstrapping transcript...", zap.Int("attempt", attempt))
		transcript, err := b.vexa.GetTranscript(ctx, b.cfg.Platform, b.cfg.NativeMeetingID)
		if err != nil {
			b.log.Warn("bootstrap failed (meeting may not be active yet)", zap.Error(err))
		} else {
			for _, seg := range transcript.Segments {
				if seg.AbsoluteStartTime != "" && strings.TrimSpace(seg.Text) != "" {
					b.segments[seg.AbsoluteStartTime] = seg
					b.agent.AddTranscript(formatSegment(seg))
				}
			}
			b.log.Info("bootstrap complete", zap.Int("segments", len(b.segments)))
		}

		// Connect WebSocket
		b.log.Info("connecting to Vexa WebSocket...")
		events, err := b.ws.Connect(ctx)
		if err != nil {
			b.log.Warn("ws connect failed, retrying...", zap.Error(err), zap.Int("attempt", attempt))
			if waitErr := b.backoff(ctx, attempt); waitErr != nil {
				return nil, waitErr
			}
			continue
		}

		b.log.Info("connected, listening for transcripts...")
		return events, nil
	}
}

func (b *Bot) backoff(ctx context.Context, attempt int) error {
	delay := time.Duration(attempt) * 5 * time.Second
	if delay > 30*time.Second {
		delay = 30 * time.Second
	}
	b.log.Info("waiting before retry...", zap.Duration("delay", delay))
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-time.After(delay):
		return nil
	}
}

func (b *Bot) handleEvent(ctx context.Context, event vexa.WSEvent, speaking *atomic.Bool) {
	switch event.Type {
	case vexa.EventTranscriptMutable:
		for _, seg := range event.Segments {
			if seg.AbsoluteStartTime == "" || strings.TrimSpace(seg.Text) == "" {
				continue
			}

			// Dedup: keep newer by updated_at
			if existing, ok := b.segments[seg.AbsoluteStartTime]; ok {
				if existing.UpdatedAt != "" && seg.UpdatedAt != "" && seg.UpdatedAt < existing.UpdatedAt {
					continue
				}
			}

			b.segments[seg.AbsoluteStartTime] = seg
			formatted := formatSegment(seg)
			b.agent.AddTranscript(formatted)
			b.log.Info("transcript", zap.String("speaker", seg.Speaker), zap.String("text", seg.Text))

			if b.broadcast != nil {
				b.broadcast(formatted)
			}

			// Check trigger
			if speaking.Load() {
				continue
			}
			question, triggered := b.agent.ShouldRespond(seg.Text)
			if !triggered {
				continue
			}

			speaking.Store(true)
			go func(q string) {
				defer speaking.Store(false)

				b.log.Info("responding to question", zap.String("question", q))

				answer, err := b.agent.Respond(ctx, q)
				if err != nil {
					b.log.Error("LLM error", zap.Error(err))
					return
				}
				b.log.Info("answer", zap.String("text", answer))

				if b.broadcast != nil {
					b.broadcast(b.cfg.BotDisplayName + ": " + answer)
				}

				if err := b.vexa.Speak(ctx, b.cfg.Platform, b.cfg.NativeMeetingID, vexa.SpeakRequest{
					Text:     answer,
					Provider: b.cfg.TTSProvider,
					Voice:    b.cfg.TTSVoice,
				}); err != nil {
					b.log.Error("speak error", zap.Error(err))
				}
			}(question)
		}

	case vexa.EventSpeakCompleted, vexa.EventSpeakInterrupted:
		speaking.Store(false)
		b.log.Info("speak finished", zap.String("event", event.Type))

	case vexa.EventMeetingStatus:
		b.log.Info("meeting status", zap.String("status", event.Status))
		if event.Status == "completed" {
			b.log.Info("meeting ended")
			b.generateFinalSummary()
		}

	case vexa.EventError:
		b.log.Error("vexa error", zap.String("error", event.Error))
	}
}

func (b *Bot) generateFinalSummary() {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	summary, err := b.agent.Summary(ctx)
	if err != nil {
		b.log.Error("final summary error", zap.Error(err))
		return
	}
	b.log.Info("FINAL MEETING SUMMARY\n" + summary)
}

func formatSegment(seg vexa.Segment) string {
	if seg.Speaker != "" {
		return seg.Speaker + ": " + seg.Text
	}
	return seg.Text
}
