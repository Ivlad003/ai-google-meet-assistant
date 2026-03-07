package vexa

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"time"

	"go.uber.org/zap"
)

type Client struct {
	baseURL    string
	apiKey     string
	httpClient *http.Client
	log        *zap.Logger
}

func NewClient(baseURL, apiKey string, log *zap.Logger) *Client {
	return &Client{
		baseURL:    baseURL,
		apiKey:     apiKey,
		httpClient: &http.Client{Timeout: 30 * time.Second},
		log:        log,
	}
}

// Segment represents a single transcript segment from Vexa.
type Segment struct {
	Text              string `json:"text"`
	Speaker           string `json:"speaker"`
	Language          string `json:"language"`
	AbsoluteStartTime string `json:"absolute_start_time"`
	AbsoluteEndTime   string `json:"absolute_end_time"`
	UpdatedAt         string `json:"updated_at"`
}

// TranscriptResponse is the response from GET /transcripts/{platform}/{meeting_id}.
type TranscriptResponse struct {
	ID              int       `json:"id"`
	Platform        string    `json:"platform"`
	NativeMeetingID string    `json:"native_meeting_id"`
	Status          string    `json:"status"`
	Segments        []Segment `json:"segments"`
}

// GetTranscript fetches the current transcript for bootstrap.
func (c *Client) GetTranscript(ctx context.Context, platform, meetingID string) (*TranscriptResponse, error) {
	url := fmt.Sprintf("%s/transcripts/%s/%s", c.baseURL, platform, meetingID)
	req, err := http.NewRequestWithContext(ctx, "GET", url, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("X-API-Key", c.apiKey)

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		body, _ := io.ReadAll(resp.Body)
		return nil, fmt.Errorf("GET transcript: status %d: %s", resp.StatusCode, body)
	}

	var result TranscriptResponse
	if err := json.NewDecoder(resp.Body).Decode(&result); err != nil {
		return nil, fmt.Errorf("decode transcript: %w", err)
	}
	return &result, nil
}

// SpeakRequest is the body for POST /bots/{platform}/{meeting_id}/speak.
type SpeakRequest struct {
	Text     string `json:"text"`
	Provider string `json:"provider"`
	Voice    string `json:"voice"`
}

// Speak sends text for the bot to speak via TTS.
func (c *Client) Speak(ctx context.Context, platform, meetingID string, req SpeakRequest) error {
	url := fmt.Sprintf("%s/bots/%s/%s/speak", c.baseURL, platform, meetingID)
	body, err := json.Marshal(req)
	if err != nil {
		return err
	}

	httpReq, err := http.NewRequestWithContext(ctx, "POST", url, bytes.NewReader(body))
	if err != nil {
		return err
	}
	httpReq.Header.Set("Content-Type", "application/json")
	httpReq.Header.Set("X-API-Key", c.apiKey)

	resp, err := c.httpClient.Do(httpReq)
	if err != nil {
		return err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK && resp.StatusCode != http.StatusAccepted {
		respBody, _ := io.ReadAll(resp.Body)
		return fmt.Errorf("speak: status %d: %s", resp.StatusCode, respBody)
	}
	return nil
}

// StopSpeaking stops any ongoing speech.
func (c *Client) StopSpeaking(ctx context.Context, platform, meetingID string) error {
	url := fmt.Sprintf("%s/bots/%s/%s/speak", c.baseURL, platform, meetingID)
	req, err := http.NewRequestWithContext(ctx, "DELETE", url, nil)
	if err != nil {
		return err
	}
	req.Header.Set("X-API-Key", c.apiKey)

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	return nil
}
