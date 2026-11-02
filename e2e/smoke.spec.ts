import { expect, test } from '@playwright/test';
import { mkdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { type RunningApp, launchApp, trackViolations } from './app';

let app: RunningApp;

test.beforeEach(async () => {
  app = await launchApp();
});

test.afterEach(async () => {
  await app.close();
});

test('starts on the chat screen with the sidebar, window controls and bundled fonts', async () => {
  const { page } = app;
  const violations = await trackViolations(page);

  await expect(page.getByRole('heading', { name: "What's on your mind?" })).toBeVisible();
  await expect(page.getByRole('complementary', { name: 'Sidebar' })).toBeVisible();
  for (const name of ['Minimize', 'Maximize', 'Close']) {
    await expect(page.getByRole('button', { name })).toBeVisible();
  }
  const interLoaded = await page.evaluate(async () => {
    await document.fonts.ready;
    return [...document.fonts].some((f) => f.family.includes('Inter') && f.status === 'loaded');
  });
  expect(interLoaded).toBe(true);
  expect(await violations()).toEqual([]);
});

test('the theme setting switches to dark and is saved', async () => {
  const { page } = app;
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('radio', { name: 'Dark' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect
    .poll(() => JSON.parse(readFileSync(join(app.dataDir, 'settings.json'), 'utf8')).theme)
    .toBe('dark');

  await page.getByRole('radio', { name: 'Light' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
});

test('Ctrl+B hides and shows the sidebar', async () => {
  const { page } = app;
  const sidebar = page.getByRole('complementary', { name: 'Sidebar' });
  await expect(sidebar).toBeVisible();
  await page.keyboard.press('Control+B');
  await expect(sidebar).toHaveCount(0);
  await page.getByRole('button', { name: 'Show sidebar' }).click();
  await expect(sidebar).toBeVisible();
});

test('the page cannot reach the network', async () => {
  const { page } = app;
  const violations = await trackViolations(page);
  // WebView2 still reports a request the content policy refused (as failed), so the check is
  // that nothing outside the app ever completes.
  const reached: string[] = [];
  page.on('requestfinished', (request) => {
    const host = new URL(request.url()).hostname;
    if (!['localhost', 'tauri.localhost', 'ipc.localhost'].includes(host))
      reached.push(request.url());
  });

  const results = await page.evaluate(async () => {
    const attempt = async (run: () => Promise<unknown>) => {
      try {
        await run();
        return 'reached';
      } catch {
        return 'blocked';
      }
    };
    const image = () =>
      new Promise((resolve, reject) => {
        const img = new Image();
        img.onload = resolve;
        img.onerror = reject;
        img.src = 'https://example.com/pixel.png';
      });
    const socket = () =>
      new Promise((resolve, reject) => {
        const ws = new WebSocket('wss://example.com/');
        ws.onopen = resolve;
        ws.onerror = reject;
      });
    return {
      fetch: await attempt(() => fetch('https://example.com/')),
      image: await attempt(image),
      socket: await attempt(socket),
      popup: window.open('https://example.com/') === null ? 'blocked' : 'reached',
      webrtc:
        typeof (window as unknown as Record<string, unknown>).RTCPeerConnection === 'undefined'
          ? 'blocked'
          : 'reached',
    };
  });

  expect(results).toEqual({
    fetch: 'blocked',
    image: 'blocked',
    socket: 'blocked',
    popup: 'blocked',
    webrtc: 'blocked',
  });
  expect(reached).toEqual([]);
  // The content policy itself refused them (the dead-end proxy is only a backstop).
  const refused = (await violations()).join('\n');
  expect(refused).toContain('img-src https://example.com/pixel.png');
  expect(refused).toContain('connect-src https://example.com/');
});

test('screenshots of every screen in light and dark, for visual review', async () => {
  const { page } = app;
  const dir = join('test-results', 'screens');
  mkdirSync(dir, { recursive: true });
  await page.setViewportSize({ width: 1280, height: 800 }).catch(() => {});
  for (const theme of ['Light', 'Dark']) {
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('radio', { name: theme }).click();
    for (const [name, open] of [
      ['chat', () => page.getByRole('radio', { name: 'Chat' }).click()],
      ['code', () => page.getByRole('radio', { name: 'Code' }).click()],
      ['models', () => page.getByRole('button', { name: 'Models' }).click()],
      ['settings', () => page.getByRole('button', { name: 'Settings', exact: true }).click()],
    ] as const) {
      await open();
      await page.waitForTimeout(150);
      await page.screenshot({ path: join(dir, `m0-${name}-${theme.toLowerCase()}.png`) });
    }
  }
});
