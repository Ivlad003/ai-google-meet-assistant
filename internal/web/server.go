package web

import (
	"context"
	"embed"
	"encoding/json"
	"io"
	"net/http"
	"strings"
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
	cfg   *config.Config
	agent *llm.Agent
	vexa  *vexa.Client
	log   *zap.Logger

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
