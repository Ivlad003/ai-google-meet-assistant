import { Page } from "playwright";
import { log, randomDelay, callJoiningCallback } from "../../utils";
import { BotConfig } from "../../types";
import { 
  googleNameInputSelectors,
  googleJoinButtonSelectors,
  googleMicrophoneButtonSelectors,
  googleCameraButtonSelectors
} from "./selectors";

export async function joinGoogleMeeting(
  page: Page,
  meetingUrl: string,
  botName: string,
  botConfig: BotConfig
): Promise<void> {
  // Install RTCPeerConnection hook before page loads - captures remote audio tracks
  // into hidden <audio> elements so BrowserAudioService can find them.
  await page.addInitScript(() => {
    try {
      const win = window as any;
      if (win.__vexaRemoteAudioHookInstalled || typeof RTCPeerConnection !== 'function') {
        return;
      }

      win.__vexaRemoteAudioHookInstalled = true;
      win.__vexaInjectedAudioElements = win.__vexaInjectedAudioElements || [];
      win.__vexaPeerConnections = win.__vexaPeerConnections || [];
      const OriginalPC = RTCPeerConnection;

      function wrapPeerConnection(this: any, ...args: any[]) {
        const pc: RTCPeerConnection = new (OriginalPC as any)(...args);
        win.__vexaPeerConnections.push(pc);

        const handleTrack = (event: RTCTrackEvent) => {
          try {
            if (!event.track || event.track.kind !== 'audio') {
              return;
            }

            const stream = (event.streams && event.streams[0]) || new MediaStream([event.track]);

            const audioEl = document.createElement('audio');
            audioEl.autoplay = true;
            audioEl.muted = false;
            audioEl.volume = 1.0;
            audioEl.dataset.vexaInjected = 'true';
            audioEl.style.position = 'absolute';
            audioEl.style.left = '-9999px';
            audioEl.style.width = '1px';
            audioEl.style.height = '1px';
            audioEl.srcObject = stream;
            audioEl.play?.().catch(() => {});

            if (document.body) {
              document.body.appendChild(audioEl);
            } else {
              document.addEventListener('DOMContentLoaded', () => document.body?.appendChild(audioEl), { once: true });
            }

            (win.__vexaInjectedAudioElements as HTMLAudioElement[]).push(audioEl);
            win.__vexaCapturedRemoteAudioStreams = win.__vexaCapturedRemoteAudioStreams || [];
            win.__vexaCapturedRemoteAudioStreams.push(stream);

            win.logBot?.(`[Audio Hook] Injected remote audio element (track=${event.track.id}, readyState=${event.track.readyState}).`);
          } catch (hookError) {
            console.error('Vexa audio hook error:', hookError);
          }
        };

        pc.addEventListener('track', handleTrack);

        const originalOnTrack = Object.getOwnPropertyDescriptor(OriginalPC.prototype, 'ontrack');
        if (originalOnTrack && originalOnTrack.set) {
          Object.defineProperty(pc, 'ontrack', {
            set(handler: any) {
              if (typeof handler !== 'function') {
                return originalOnTrack.set!.call(this, handler);
              }
              const wrapped = function (this: RTCPeerConnection, event: RTCTrackEvent) {
                handleTrack(event);
                return handler.call(this, event);
              };
              return originalOnTrack.set!.call(this, wrapped);
            },
            get: originalOnTrack.get,
            configurable: true,
            enumerable: true
          });
        }

        return pc;
      }

      wrapPeerConnection.prototype = OriginalPC.prototype;
      Object.setPrototypeOf(wrapPeerConnection, OriginalPC);
      (window as any).RTCPeerConnection = wrapPeerConnection as any;

      win.logBot?.('[Audio Hook] RTCPeerConnection patched to mirror remote audio tracks.');
    } catch (initError) {
      console.error('Failed to install Vexa audio hook:', initError);
    }
  });

  await page.goto(meetingUrl, { waitUntil: "networkidle" });
  await page.bringToFront();

  // Take screenshot after navigation
  try { await page.screenshot({ path: '/app/storage/screenshots/bot-checkpoint-0-after-navigation.png', fullPage: true }); } catch {}
  log("Screenshot: After navigation to meeting URL");
  
  // --- Call joining callback to notify bot-manager that bot is joining ---
  try {
    await callJoiningCallback(botConfig);
    log("Joining callback sent successfully");
  } catch (callbackError: any) {
    log(`Warning: Failed to send joining callback: ${callbackError.message}. Continuing with join process...`);
  }

  // Add a longer, fixed wait after navigation for page elements to settle
  log("Waiting for page elements to settle after navigation...");
  await page.waitForTimeout(5000); // Wait 5 seconds

  // Enter name and join
  await page.waitForTimeout(randomDelay(1000));
  log("Attempting to find name input field...");
  
  // Use selector from selectors.ts instead of inline
  const nameFieldSelector = googleNameInputSelectors[0];
  await page.waitForSelector(nameFieldSelector, { timeout: 120000 }); // 120 seconds
  log("Name input field found.");
  
  // Take screenshot after finding name field
  try { await page.screenshot({ path: '/app/storage/screenshots/bot-checkpoint-0-name-field-found.png', fullPage: true }); } catch {}
  log("Screenshot: Name input field found");

  await page.waitForTimeout(randomDelay(1000));
  await page.fill(nameFieldSelector, botName);

  // Mute mic and camera if available
  try {
    await page.waitForTimeout(randomDelay(500));
    const micSelector = googleMicrophoneButtonSelectors[0];
    await page.click(micSelector, { timeout: 200 });
    await page.waitForTimeout(200);
  } catch (e) {
    log("Microphone already muted or not found.");
  }
  
  try {
    await page.waitForTimeout(randomDelay(500));
    const cameraSelector = googleCameraButtonSelectors[0];
    await page.click(cameraSelector, { timeout: 200 });
    await page.waitForTimeout(200);
  } catch (e) {
    log("Camera already off or not found.");
  }

  // Try all join button selectors until one works
  let joinClicked = false;
  for (const selector of googleJoinButtonSelectors) {
    try {
      await page.waitForSelector(selector, { timeout: 10000 });
      await page.click(selector);
      log(`Clicked join button with selector: ${selector}`);
      joinClicked = true;
      break;
    } catch {
      log(`Join selector not found: ${selector}`);
    }
  }
  if (!joinClicked) {
    throw new Error("Could not find any join button");
  }
  log(`${botName} joined the Google Meet Meeting.`);
  
  // Take screenshot after clicking "Ask to join"
  try { await page.screenshot({ path: '/app/storage/screenshots/bot-checkpoint-0-after-ask-to-join.png', fullPage: true }); } catch {}
  log("Screenshot: After clicking 'Ask to join'");
}
