import WebSocket from 'ws';
import { log } from '../utils';

/**
 * BridgeClient — connects vexa-bot to the local Rust WebSocket server
 * instead of Redis + WhisperLive when running in desktop/bridge mode.
 *
 * Protocol:
 *   Bot → Rust:
 *     - Binary frames: raw Float32 PCM audio (16 kHz mono)
 *     - JSON text frames: { type: "event", event: "...", data: {...} }
 *
 *   Rust → Bot:
 *     - { type: "command", action: "...", ... }   (same shape as Redis commands)
 *     - { type: "speak",   audio: "<base64wav>" } (pre-rendered TTS audio)
 */
export class BridgeClient {
  private ws: WebSocket | null = null;
  private url: string;
  private onCommand: ((action: string, data: any) => void) | null = null;
  private onSpeakAudio: ((audioBase64: string) => void) | null = null;
  private reconnectTimer: NodeJS.Timeout | null = null;
  private closed: boolean = false;

  constructor(url: string = 'ws://localhost:9090/ws') {
    this.url = url;
  }

  /**
   * Connect to the Rust bridge WebSocket server.
   * Resolves on first successful open; auto-reconnects on subsequent drops.
   */
  connect(): Promise<void> {
    return new Promise((resolve, reject) => {
      let resolved = false;
      try {
        this.ws = new WebSocket(this.url);
      } catch (err) {
        reject(err);
        return;
      }

      this.ws.on('open', () => {
        log('[Bridge] Connected to Rust core');
        if (!resolved) {
          resolved = true;
          resolve();
        }
      });

      this.ws.on('message', (data: Buffer | string) => {
        try {
          const msg = JSON.parse(data.toString());
          if (msg.type === 'speak' && this.onSpeakAudio) {
            this.onSpeakAudio(msg.audio);
          } else if (msg.type === 'command' && this.onCommand) {
            this.onCommand(msg.action, msg);
          }
        } catch (e: any) {
          log(`[Bridge] Parse error: ${e.message}`);
        }
      });

      this.ws.on('close', () => {
        if (this.closed) return;
        log('[Bridge] Disconnected, reconnecting in 3s...');
        this.scheduleReconnect();
      });

      this.ws.on('error', (err: Error) => {
        log(`[Bridge] WebSocket error: ${err.message}`);
        if (!resolved) {
          resolved = true;
          reject(err);
        }
        // on('close') will fire after this and handle reconnection
      });
    });
  }

  /**
   * Send raw Float32 PCM audio as a binary frame.
   * The Rust side receives this as raw bytes and converts to f32 slices.
   */
  sendAudio(float32Data: Float32Array): boolean {
    if (this.ws?.readyState === WebSocket.OPEN) {
      try {
        // Send the underlying ArrayBuffer as a Node.js Buffer (binary frame)
        this.ws.send(Buffer.from(float32Data.buffer, float32Data.byteOffset, float32Data.byteLength));
        return true;
      } catch (err: any) {
        log(`[Bridge] Error sending audio: ${err.message}`);
        return false;
      }
    }
    return false;
  }

  /**
   * Send a JSON event to the Rust core.
   * e.g. sendEvent('bot.joined', { meetingUrl: '...' })
   */
  sendEvent(event: string, data: any = {}): boolean {
    if (this.ws?.readyState === WebSocket.OPEN) {
      try {
        this.ws.send(JSON.stringify({ type: 'event', event, data }));
        return true;
      } catch (err: any) {
        log(`[Bridge] Error sending event: ${err.message}`);
        return false;
      }
    }
    return false;
  }

  /**
   * Register handler for command messages from Rust.
   * Commands have the same shape as Redis commands: { action: 'speak' | 'leave' | ... }
   */
  onCommandReceived(handler: (action: string, data: any) => void): void {
    this.onCommand = handler;
  }

  /**
   * Register handler for speak messages (pre-rendered TTS audio from Rust).
   * The audio field is base64-encoded WAV.
   */
  onSpeakReceived(handler: (audioBase64: string) => void): void {
    this.onSpeakAudio = handler;
  }

  /**
   * Check if the WebSocket is currently connected and open.
   */
  isConnected(): boolean {
    return this.ws?.readyState === WebSocket.OPEN;
  }

  /**
   * Close connection and stop reconnection attempts.
   */
  close(): void {
    this.closed = true;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.ws) {
      try {
        this.ws.close();
      } catch {}
      this.ws = null;
    }
  }

  private scheduleReconnect(): void {
    if (this.closed) return;
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = setTimeout(async () => {
      this.reconnectTimer = null;
      try {
        await this.connect();
      } catch (err: any) {
        log(`[Bridge] Reconnect failed: ${err.message}. Will retry...`);
        // connect() failure triggers on('error') but not on('close'),
        // so schedule another attempt manually.
        this.scheduleReconnect();
      }
    }, 3000);
  }
}
