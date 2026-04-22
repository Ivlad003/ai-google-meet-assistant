// @ts-check
const { test, expect } = require('@playwright/test');

const BASE = process.env.TEST_URL || 'http://localhost:8090';

test.describe('Jarvis Web UI', () => {

  test('Dashboard tab loads with all fields', async ({ page }) => {
    await page.goto(BASE);
    await expect(page.locator('h1')).toHaveText('Jarvis');

    // Tab nav exists with 3 tabs
    const tabs = page.locator('.tab-nav button');
    await expect(tabs).toHaveCount(3);
    await expect(tabs.nth(0)).toHaveText('Dashboard');
    await expect(tabs.nth(1)).toHaveText('Sessions');
    await expect(tabs.nth(2)).toHaveText('Tools');

    // Dashboard is active by default
    await expect(page.locator('#tab-dashboard')).toHaveClass(/active/);

    // Settings fields exist
    await expect(page.locator('#meet-url')).toBeVisible();
    await expect(page.locator('#bot-name')).toBeVisible();
    await expect(page.locator('#language')).toBeVisible();
    await expect(page.locator('#tts-voice')).toBeVisible();
    await expect(page.locator('#model')).toBeVisible();
    await expect(page.locator('#response-mode')).toBeVisible();
    await expect(page.locator('#record-video')).toBeVisible();

    // Language has correct options
    const langOptions = page.locator('#language option');
    await expect(langOptions).toHaveCount(3);

    // Buttons exist
    await expect(page.getByText('Save & Reload')).toBeVisible();
    await expect(page.getByText('Join Meeting')).toBeVisible();
    await expect(page.getByText('Leave Meeting')).toBeVisible();
  });

  test('Sessions tab loads and shows session list', async ({ page }) => {
    await page.goto(BASE);

    // Switch to Sessions tab and wait for load
    const sessionsPromise = page.waitForResponse(resp => resp.url().includes('/api/sessions') && resp.status() === 200);
    await page.locator('.tab-nav button[data-tab="sessions"]').click();
    await expect(page.locator('#tab-sessions')).toHaveClass(/active/);
    await sessionsPromise;
    await page.waitForTimeout(500);

    // Session list should have items
    const items = page.locator('.session-item');
    const count = await items.count();
    expect(count).toBeGreaterThan(0);

    // First item has date and preview
    await expect(items.first().locator('.date')).toBeVisible();
    await expect(items.first().locator('.badges')).toBeVisible();

    // Search input exists
    await expect(page.locator('#session-search')).toBeVisible();

    // Global chat section exists
    await expect(page.locator('.global-chat-bar h2')).toBeVisible();
  });

  test('Session detail opens with transcript and audio', async ({ page }) => {
    const sessionsPromise = page.waitForResponse(resp => resp.url().includes('/api/sessions') && resp.status() === 200);
    await page.goto(BASE + '#sessions');
    await sessionsPromise;
    await page.waitForTimeout(500);

    // Click first session
    const firstItem = page.locator('.session-item').first();
    const sessionId = await firstItem.getAttribute('data-sid');
    await firstItem.click();

    // Wait for transcript to load
    await page.waitForTimeout(1000);

    // Detail panel should show content
    const detail = page.locator('#session-detail');
    await expect(detail).not.toHaveClass(/session-detail-empty/);

    // Transcript viewer exists
    await expect(page.locator('.transcript-viewer')).toBeVisible();

    // Audio player exists
    await expect(page.locator('audio')).toBeVisible();

    // Download buttons exist
    await expect(page.locator('text=Transcript (.txt)')).toBeVisible();
    await expect(page.locator('text=Audio (.wav)')).toBeVisible();

    // Delete button exists
    await expect(page.locator('text=Delete Session')).toBeVisible();

    // Chat input exists
    await expect(page.locator('#chat-input')).toBeVisible();
  });

  test('Global chat expands and has input', async ({ page }) => {
    await page.goto(BASE + '#sessions');
    await page.waitForTimeout(500);

    // Click global chat header to expand
    await page.locator('.global-chat-bar h2').click();

    // Chat body should be visible
    await expect(page.locator('#global-chat-body')).toHaveClass(/open/);
    await expect(page.locator('#global-chat-input')).toBeVisible();
  });

  test('Tools tab loads with editor and AI chat', async ({ page }) => {
    await page.goto(BASE);

    // Switch to Tools tab
    await page.locator('.tab-nav button[data-tab="tools"]').click();
    await expect(page.locator('#tab-tools')).toHaveClass(/active/);

    // Wait for tools config to load
    await page.waitForTimeout(1000);

    // JSON editor exists
    await expect(page.locator('#tools-json')).toBeVisible();

    // Save and Format buttons exist
    await expect(page.getByText('Save Tools')).toBeVisible();
    await expect(page.getByText('Format JSON')).toBeVisible();

    // AI chat input exists
    await expect(page.locator('#tools-chat-input')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Generate' })).toBeVisible();
  });

  test('Search finds results', async ({ page }) => {
    await page.goto(BASE + '#sessions');
    await page.waitForResponse(resp => resp.url().includes('/api/sessions') && resp.status() === 200);
    await page.waitForTimeout(500);

    // Type search query
    await page.locator('#session-search').fill('Влад');
    await page.locator('#session-search').press('Enter');

    // Wait for search results
    await page.waitForResponse(resp => resp.url().includes('/api/sessions/search') && resp.status() === 200);
    await page.waitForTimeout(500);

    // Should show search results with highlighting
    const results = page.locator('.search-result-session');
    const count = await results.count();
    expect(count).toBeGreaterThan(0);

    // Should have highlighted matches
    await expect(page.locator('mark').first()).toBeVisible();
  });

  test('Tab switching preserves state', async ({ page }) => {
    await page.goto(BASE);

    // Go to Sessions
    await page.locator('.tab-nav button[data-tab="sessions"]').click();
    await expect(page.locator('#tab-sessions')).toHaveClass(/active/);
    await expect(page.locator('#tab-dashboard')).not.toHaveClass(/active/);

    // Go to Tools
    await page.locator('.tab-nav button[data-tab="tools"]').click();
    await expect(page.locator('#tab-tools')).toHaveClass(/active/);
    await expect(page.locator('#tab-sessions')).not.toHaveClass(/active/);

    // Back to Dashboard
    await page.locator('.tab-nav button[data-tab="dashboard"]').click();
    await expect(page.locator('#tab-dashboard')).toHaveClass(/active/);
    await expect(page.locator('#tab-tools')).not.toHaveClass(/active/);
  });

  test('Config saves language and record_video', async ({ page }) => {
    await page.goto(BASE);
    await page.waitForTimeout(500);

    // Change language to Ukrainian
    await page.locator('#language').selectOption('uk');

    // Check record video
    await page.locator('#record-video').check();

    // Save
    await page.getByText('Save & Reload').click();

    // Wait for toast
    await expect(page.locator('#toast')).toBeVisible({ timeout: 3000 });

    // Reload and verify
    await page.reload();
    await page.waitForTimeout(1000);
    await expect(page.locator('#language')).toHaveValue('uk');
    await expect(page.locator('#record-video')).toBeChecked();

    // Reset
    await page.locator('#language').selectOption('auto');
    await page.locator('#record-video').uncheck();
    await page.getByText('Save & Reload').click();
    await page.waitForTimeout(500);
  });
});
