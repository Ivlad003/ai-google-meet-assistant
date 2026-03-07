package vexa

import (
	"context"
	"encoding/json"
	"net/http"
	"sync"
	"time"

	"github.com/gorilla/websocket"
	"go.uber.org/zap"
)

// Event types received on WebSocket.
const (
	EventTranscriptMutable = "transcript.mutable"
	EventMeetingStatus     = "meeting.status"
	EventSpeakStarted      = "speak.started"
	EventSpeakCompleted    = "speak.completed"
	EventSpeakInterrupted  = "speak.interrupted"
	EventSubscribed        = "subscribed"
	EventPong              = "pong"
	EventError             = "error"
)

// WSMessage is the generic WebSocket message envelope.
type WSMessage struct {
	Type    string          `json:"type"`
	Meeting json.RawMessage `json:"meeting,omitempty"`
	Payload json.RawMessage `json:"payload,omitempty"`
	TS      string          `json:"ts,omitempty"`
	Error   string          `json:"error,omitempty"`
}

// TranscriptPayload is the payload of a transcript.mutable message.
type TranscriptPayload struct {
	Segments []Segment `json:"segments"`
}

// MeetingStatusPayload is the payload of a meeting.status message.
type MeetingStatusPayload struct {
	Status string `json:"status"`
}

// WSEvent is a parsed event emitted to the bot loop.
type WSEvent struct {
	Type     string
	Segments []Segment // for transcript.mutable
	Status   string    // for meeting.status
	Error    string    // for error
}

// WSClient manages the WebSocket connection to Vexa.
type WSClient struct {
	wsURL     string
	apiKey    string
	platform  string
	meetingID string
	log       *zap.Logger

	mu   sync.Mutex
	conn *websocket.Conn
}

func NewWSClient(wsURL, apiKey, platform, meetingID string, log *zap.Logger) *WSClient {
	return &WSClient{
		wsURL:     wsURL,
		apiKey:    apiKey,
		platform:  platform,
		meetingID: meetingID,
		log:       log,
	}
}

// Connect establishes WS connection and subscribes to the meeting.
// Returns a channel of events. Handles reconnection internally.
func (ws *WSClient) Connect(ctx context.Context) (<-chan WSEvent, error) {
	events := make(chan WSEvent, 50)

	if err := ws.dial(ctx); err != nil {
		return nil, err
	}

	if err := ws.subscribe(); err != nil {
		ws.conn.Close()
		return nil, err
	}

	go ws.readLoop(ctx, events)
	go ws.pingLoop(ctx)

	return events, nil
}

func (ws *WSClient) dial(ctx context.Context) error {
	header := http.Header{}
	header.Set("X-API-Key", ws.apiKey)

	conn, _, err := websocket.DefaultDialer.DialContext(ctx, ws.wsURL, header)
	if err != nil {
		return err
	}

	ws.mu.Lock()
	ws.conn = conn
	ws.mu.Unlock()
	return nil
}

func (ws *WSClient) subscribe() error {
	msg := map[string]any{
		"action": "subscribe",
		"meetings": []map[string]string{
			{"platform": ws.platform, "native_id": ws.meetingID},
		},
	}
	ws.mu.Lock()
	defer ws.mu.Unlock()
	return ws.conn.WriteJSON(msg)
}

func (ws *WSClient) readLoop(ctx context.Context, events chan<- WSEvent) {
	defer close(events)

	for {
		select {
		case <-ctx.Done():
			return
		default:
		}

		ws.mu.Lock()
		conn := ws.conn
		ws.mu.Unlock()

		_, message, err := conn.ReadMessage()
		if err != nil {
			ws.log.Warn("ws read error, reconnecting...", zap.Error(err))
			if ctx.Err() != nil {
				return
			}
			ws.reconnect(ctx)
			continue
		}

		var msg WSMessage
		if err := json.Unmarshal(message, &msg); err != nil {
			ws.log.Warn("ws unmarshal error", zap.Error(err))
			continue
		}

		event := ws.parseMessage(msg)
		if event != nil {
			select {
			case events <- *event:
			case <-ctx.Done():
				return
			}
		}
	}
}

func (ws *WSClient) parseMessage(msg WSMessage) *WSEvent {
	switch msg.Type {
	case EventTranscriptMutable:
		var payload TranscriptPayload
		if err := json.Unmarshal(msg.Payload, &payload); err != nil {
			ws.log.Warn("unmarshal transcript payload", zap.Error(err))
			return nil
		}
		return &WSEvent{Type: EventTranscriptMutable, Segments: payload.Segments}

	case EventMeetingStatus:
		var payload MeetingStatusPayload
		if err := json.Unmarshal(msg.Payload, &payload); err != nil {
			return nil
		}
		return &WSEvent{Type: EventMeetingStatus, Status: payload.Status}

	case EventSpeakCompleted, EventSpeakInterrupted:
		return &WSEvent{Type: msg.Type}

	case EventSpeakStarted:
		return &WSEvent{Type: EventSpeakStarted}

	case EventError:
		return &WSEvent{Type: EventError, Error: msg.Error}

	case EventSubscribed, EventPong:
		ws.log.Debug("ws event", zap.String("type", msg.Type))
		return nil

	default:
		ws.log.Debug("unknown ws event", zap.String("type", msg.Type))
		return nil
	}
}

func (ws *WSClient) reconnect(ctx context.Context) {
	for attempt := 1; ; attempt++ {
		select {
		case <-ctx.Done():
			return
		case <-time.After(time.Duration(attempt) * 2 * time.Second):
		}

		ws.log.Info("reconnecting...", zap.Int("attempt", attempt))
		if err := ws.dial(ctx); err != nil {
			ws.log.Warn("reconnect dial failed", zap.Error(err))
			continue
		}
		if err := ws.subscribe(); err != nil {
			ws.log.Warn("reconnect subscribe failed", zap.Error(err))
			continue
		}
		ws.log.Info("reconnected successfully")
		return
	}
}

func (ws *WSClient) pingLoop(ctx context.Context) {
	ticker := time.NewTicker(25 * time.Second)
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			ws.mu.Lock()
			err := ws.conn.WriteJSON(map[string]string{"action": "ping"})
			ws.mu.Unlock()
			if err != nil {
				ws.log.Warn("ping failed", zap.Error(err))
			}
		}
	}
}

// Close cleanly shuts down the WebSocket connection.
func (ws *WSClient) Close() {
	ws.mu.Lock()
	defer ws.mu.Unlock()
	if ws.conn != nil {
		ws.conn.Close()
	}
}
