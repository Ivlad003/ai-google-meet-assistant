# Sessions UI — Design Document

**Date:** 2026-04-22
**Status:** Validated (post vibe-testing)

## Overview

Add a Sessions tab to the Jarvis Web UI for browsing past meetings, playing back audio/video, downloading files, and chatting with transcriptions via LLM.

## UI Layout

Tabbed SPA with two tabs:
- **Dashboard** — existing page (settings, status, live transcript, summary)
- **Sessions** — session history + detail view + AI chat

## Sessions Tab — Structure

### Session List (left panel / top)
- List of past sessions from `data_dir/sessions/`, paginated (20 per page, newest first)
- Each entry shows: date/time, preview (first speaker + first words from transcript), available file icons (txt/wav/video)
- Current live session excluded from list (files modified within last 60s) or marked with a "Live" badge
- Cross-session search bar at the top — case-insensitive full-text search across `.txt` files
- Search results show matching lines with 1 line of context before/after, limited to 100 matches total
- Empty state: friendly message "No sessions yet. Join a meeting from the Dashboard to get started."

### Session Detail (right panel / bottom)
When a session is selected:

1. **Transcript viewer** — full text of the `.txt` file, scrollable, monospace
2. **Audio player** — native HTML5 `<audio>` element for the `.wav` file
3. **Video player** — native HTML5 `<video>` element for `.webm` files only. MKV files are download-only (browsers don't support MKV playback). Hidden when no video.
4. **Download buttons** — download transcript (.txt, raw text), audio (.wav), video (.mkv/.webm)
5. **AI Chat** — chat input + message history for asking questions about the session. Frontend stores full chat history and sends it with each request for follow-up context.

## API Endpoints (new)

### `GET /api/sessions?limit=20&offset=0`
Returns paginated list of sessions with metadata.
```json
{
  "sessions": [
    {
      "id": "2026-04-22_143000",
      "date": "2026-04-22 14:30:00",
      "preview": "[John] Hello everyone, let's start the standup",
      "has_audio": true,
      "has_video": false,
      "has_transcript": true,
      "audio_size": 15234567,
      "video_size": null,
      "transcript_size": 4567,
      "video_format": null
    }
  ],
  "total": 45,
  "limit": 20,
  "offset": 0
}
```
Implementation: scan `sessions/` directory, group files by timestamp prefix, read first transcript line for preview. Exclude files modified within 60 seconds (active session).

### `GET /api/sessions/:id/transcript`
Returns transcript as JSON for the chat UI to consume.
```json
{ "text": "[14:30:05] [John] Hello everyone..." }
```

### `GET /api/sessions/:id/transcript/download`
Returns raw transcript text with proper headers for file download.
- `Content-Type: text/plain; charset=utf-8`
- `Content-Disposition: attachment; filename="2026-04-22_143000.txt"`

### `GET /api/sessions/:id/audio`
Serves the `.wav` file with `Content-Type: audio/wav`. Supports HTTP Range requests for seeking (required for large WAV files — ~115MB/hr at 16kHz mono).

### `GET /api/sessions/:id/video`
Serves the video file. Supports Range requests.
- `.webm` → `Content-Type: video/webm` (browser-playable)
- `.mkv` → `Content-Type: video/x-matroska` (download only, browser won't play)

### `POST /api/sessions/:id/chat`
Chat about a single session's transcript. Uses a **standalone LLM call** (not the live `LlmAgent`) to avoid polluting live meeting context.
```json
// Request
{
  "message": "What were the action items?",
  "history": [
    { "role": "user", "content": "Summarize the meeting" },
    { "role": "assistant", "content": "The meeting covered..." }
  ]
}
// Response
{ "reply": "Based on the transcript, the action items discussed were..." }
```
Backend: loads session transcript as system context, appends chat history + new message, calls OpenAI directly (not through `LlmAgent`).

### `POST /api/sessions/search`
Cross-session text search (case-insensitive).
```json
// Request
{ "query": "deployment", "max_results": 100 }
// Response
{
  "results": [
    {
      "session_id": "2026-04-22_143000",
      "session_date": "2026-04-22 14:30:00",
      "matches": [
        {
          "line": 42,
          "text": "[14:35:12] [John] We need to discuss the deployment timeline",
          "context_before": "[14:35:00] [Maria] Next topic?",
          "context_after": "[14:35:20] [John] The staging deploy is scheduled for Friday"
        }
      ]
    }
  ]
}
```

### `POST /api/sessions/chat`
Cross-session AI chat. Max 3 sessions, transcripts truncated to ~4000 tokens each.
```json
// Request
{
  "message": "When did we last discuss deployment?",
  "session_ids": ["2026-04-22_143000", "2026-04-20_100000"],
  "history": []
}
// Response
{ "reply": "Deployment was discussed in two recent meetings..." }
```
Backend: validates max 3 session IDs, loads and truncates each transcript, builds standalone LLM prompt (not through `LlmAgent`).

## Frontend Architecture

### Tab System
- CSS-only tab switching (no framework needed)
- Active tab highlighted, content panels shown/hidden
- URL hash for deep linking (`#dashboard`, `#sessions`, `#sessions/2026-04-22_143000`)

### Session List
- Fetched on tab switch via `GET /api/sessions`
- "Load more" button for pagination
- Search bar triggers `POST /api/sessions/search`, shows results with context
- Click session → loads detail view
- Empty search results: "No matches found for '...'"

### Session Detail
- Transcript loaded via `/api/sessions/:id/transcript`
- Audio element `src` points to `/api/sessions/:id/audio`
- Video element `src` points to `/api/sessions/:id/video` (shown only for `.webm`)
- Download buttons: transcript uses `/api/sessions/:id/transcript/download`, audio/video use their serve endpoints with `download` attribute
- MKV video: show download button only, no player

### Chat Section (within Session Detail)
- Input field + send button
- Message history rendered as user/assistant bubbles
- Frontend maintains `history[]` array, sends with each request
- "Ask about this meeting" placeholder text
- Loading spinner while waiting for LLM response

### Cross-Session Chat Flow
1. User types search query in search bar
2. Results show matching sessions with highlighted excerpts + context lines
3. User selects up to 3 sessions via checkboxes
4. Chat input appears below results, sends selected session IDs + question to `/api/sessions/chat`
5. Follow-up questions include history array

## Implementation Scope

### Backend (Rust — server.rs)
- New route group under `/api/sessions`
- Session scanner: read `sessions/` dir, parse filenames, get file sizes, read first transcript line
- Pagination support (limit/offset)
- File serving with Range request support (for audio/video seeking)
- Text search: read `.txt` files, case-insensitive grep with context lines
- Standalone LLM chat function: builds prompt with transcript context, calls OpenAI API directly (separate from live `LlmAgent`)
- Input validation: session ID format, max 3 sessions for cross-chat, max_results cap

### Frontend (index.html)
- Tab navigation system
- Sessions list with search + pagination
- Session detail view with players + downloads + chat
- Chat history management in JS
- Empty states for all views
- All in the single embedded `index.html` (no build step)

### No changes needed
- Config system (record_video flag already exists)
- Session file format (already timestamped correctly)
- Database schema (file-based approach, no DB changes)

## Non-Goals (v1)
- Transcript-synced audio playback (clicking transcript → jumps to time)
- Waveform visualization
- Embedding-based semantic search
- Session deletion from UI
- Auto-generated session summaries stored in DB
- Audio compression (serve WAV as-is; mp3/ogg conversion is future work)

## Resolved Gaps (from vibe testing)

| ID | Issue | Resolution |
|---|---|---|
| G-B1 | MKV won't play in browsers | WebM = player, MKV = download-only |
| G-B2 | Transcript download returned JSON | Added `/transcript/download` endpoint returning `text/plain` |
| G-B3 | Chat polluted live LlmAgent context | Standalone LLM calls for session chat, separate from live agent |
| G-D1 | No pagination | Added `limit`/`offset` to session list API |
| G-D2 | No token budget for cross-session chat | Max 3 sessions, ~4000 tokens each |
| G-D3 | No way to distinguish same-day sessions | Added preview (first speaker + first words) |
| G-D5 | Search had no limits or context | Added `max_results` + 1 line context before/after |
| G-D6 | Live session appeared in history | Excluded files modified within 60s |
| G-D7 | No follow-up chat support | Frontend sends `history[]` with each request |
| G-C1 | No empty states | Defined empty states for list, search, and detail |
| G-C2 | Case sensitivity unspecified | Case-insensitive search |
