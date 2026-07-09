// One-time Google login helper for the meeting bot.
//
// Opens a persistent Chrome profile at BOT_PROFILE_DIR and navigates to the
// Google sign-in page. You log in manually (incl. 2FA / "verify it's you"),
// then press Enter to save the session into the profile and close.
//
// The bot (index.ts) later reuses that same profile via BOT_PROFILE_DIR so
// Google Meet treats it as a signed-in participant instead of blocking the
// anonymous automated join.
//
//   Build:  npm run build
//   Run:    BOT_PROFILE_DIR=./chrome-profile npm run login
//
// Do the login on the SAME machine/network that will run the bot when
// possible — a session created on a home IP and used from a datacenter IP
// may trigger Google's re-verification.

import { chromium } from "playwright-extra";
import StealthPlugin from "puppeteer-extra-plugin-stealth";
import * as fs from "fs";
import * as readline from "readline";

(async () => {
  const profileDir = (process.env.BOT_PROFILE_DIR || "./chrome-profile").trim();
  const headless = process.env.HEADLESS === "true" || process.env.HEADLESS === "1";

  const stealthPlugin = StealthPlugin();
  stealthPlugin.enabledEvasions.delete("iframe.contentWindow");
  stealthPlugin.enabledEvasions.delete("media.codecs");
  chromium.use(stealthPlugin);

  fs.mkdirSync(profileDir, { recursive: true });

  console.log("\n=== Google account login for the meeting bot ===");
  console.log(`Profile dir: ${profileDir}`);
  if (headless) {
    console.log("WARNING: HEADLESS is set — Google login usually needs a visible window.");
  }

  const context = await chromium.launchPersistentContext(profileDir, {
    headless,
    args: ["--no-sandbox", "--disable-blink-features=AutomationControlled"],
    viewport: { width: 1280, height: 800 },
  });

  const page = context.pages()[0] || (await context.newPage());
  await page.goto("https://accounts.google.com/", { waitUntil: "load" }).catch(() => {});

  console.log("\nA Chrome window opened. Log in to the DEDICATED bot Google account.");
  console.log("Complete any 2FA / 'verify it's you' steps until the account is signed in.");
  console.log("Then return here and press Enter to save the session and close.\n");

  await new Promise<void>((resolve) => {
    const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
    rl.question("Press Enter when logged in... ", () => {
      rl.close();
      resolve();
    });
  });

  await context.close();
  console.log(`\nSaved. Logged-in session stored in: ${profileDir}`);
  console.log(`Point the bot at it with BOT_PROFILE_DIR=${profileDir}\n`);
  process.exit(0);
})().catch((e) => {
  console.error("Login helper error:", e);
  process.exit(1);
});
