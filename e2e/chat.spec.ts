import { type Page, expect, test } from '@playwright/test';
import { mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { type RunningApp, launchApp, trackViolations } from './app';
import { type MockLlm, startMock } from './mock';

let mock: MockLlm;
let app: RunningApp;
let saveDir: string;

test.beforeAll(async () => {
  mock = await startMock();
});

test.afterAll(() => {
  mock.stop();
});

test.beforeEach(async () => {
  saveDir = mkdtempSync(join(tmpdir(), 'hearth-save-'));
  app = await launchApp({
    env: { HEARTH_TEST_LLM_URL: mock.url, HEARTH_TEST_SAVE_PATH: join(saveDir, 'export.md') },
  });
});

test.afterEach(async () => {
  await app.close();
  rmSync(saveDir, { recursive: true, force: true });
});

const RICH = [
  '## Results',
  '',
  '| Size | Time |',
  '| --- | --- |',
  '| 1k | 2 ms |',
  '',
  '```python',
  'print("hi")',
  '```',
  '',
  'Euler: $e^{i\\pi} + 1 = 0$',
].join('\n');

async function send(page: Page, text: string) {
  const box = page.getByRole('textbox', { name: 'Message' });
  await box.fill(text);
  await box.press('Enter');
}

const lastReply = (page: Page) => page.locator('[data-role="assistant"]').last();

test('a reply streams in with markdown, code, math and its thinking', async () => {
  await mock.setScript({
    steps: [
      {
        reply: {
          content: RICH,
          reasoning: 'Let me lay this out.',
          stream: { chunk_chars: 6, delay_ms: 5 },
        },
      },
    ],
  });
  const { page } = app;
  const violations = await trackViolations(page);
  await send(page, 'Show me a summary');

  const reply = lastReply(page);
  await expect(reply.locator('table')).toBeVisible();
  await expect(reply.locator('.katex').first()).toBeVisible();
  await expect(reply.locator('pre').first()).toContainText('print("hi")');
  const thoughts = reply.getByRole('button', { name: /Thought for/ });
  await expect(thoughts).toBeVisible();
  await thoughts.click();
  await expect(reply.getByText('Let me lay this out.')).toBeVisible();
  await expect(
    page
      .getByRole('navigation', { name: 'Conversations' })
      .getByRole('button', { name: 'Show me a summary', exact: true }),
  ).toBeVisible();
  expect(await violations()).toEqual([]);

  const sent = await mock.requests();
  expect(sent.at(-1)?.messages.at(-1)).toEqual({ role: 'user', content: 'Show me a summary' });
});

test('stop keeps what was written', async () => {
  await mock.setScript({
    steps: [{ reply: { content: 'Partial reply here', stream: { chunk_chars: 4, hold: 'end' } } }],
  });
  const { page } = app;
  await send(page, 'Go');
  await expect(lastReply(page)).toContainText('Partial reply here');
  await page.getByRole('button', { name: 'Stop' }).click();
  await expect(lastReply(page).getByText('Stopped')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Send' })).toBeVisible();
  await mock.release('end');
});

test('regenerate makes a second version and you can switch back', async () => {
  await mock.setScript({
    steps: [{ reply: { content: 'First take' } }, { reply: { content: 'Second take' } }],
  });
  const { page } = app;
  await send(page, 'Hello');
  await expect(lastReply(page)).toContainText('First take');
  await lastReply(page).getByRole('button', { name: 'Regenerate' }).click();
  await expect(lastReply(page)).toContainText('Second take');
  await expect(lastReply(page).getByText('2 / 2')).toBeVisible();
  await lastReply(page).getByRole('button', { name: 'Previous version' }).click();
  await expect(lastReply(page)).toContainText('First take');
  await expect(lastReply(page).getByText('1 / 2')).toBeVisible();
});

test('editing a message starts a new branch', async () => {
  await mock.setScript({
    steps: [{ reply: { content: 'Reply A' } }, { reply: { content: 'Reply B' } }],
  });
  const { page } = app;
  await send(page, 'Hi');
  await expect(lastReply(page)).toContainText('Reply A');

  const user = page.locator('[data-role="user"]').first();
  await user.getByRole('button', { name: 'Edit' }).click();
  const editor = page.getByRole('textbox', { name: 'Edit message' });
  await editor.fill('Hi again');
  await editor.press('Enter');

  await expect(lastReply(page)).toContainText('Reply B');
  await expect(page.locator('[data-role="user"]').first()).toContainText('Hi again');
  await expect(page.locator('[data-role="user"]').first().getByText('2 / 2')).toBeVisible();
  const sent = await mock.requests();
  expect(sent.at(-1)?.messages.at(-1)).toEqual({ role: 'user', content: 'Hi again' });
});

test('search finds a conversation by what was said', async () => {
  await mock.setScript({ steps: [{ reply: { content: 'The answer involves quokkas.' } }] });
  const { page } = app;
  await send(page, 'Tell me about animals');
  await expect(lastReply(page)).toContainText('quokkas');
  await expect(page.getByRole('button', { name: 'Regenerate' })).toBeVisible();

  await page.keyboard.press('Control+N');
  await expect(page.getByRole('heading', { name: "What's on your mind?" })).toBeVisible();
  await page.getByRole('textbox', { name: 'Search conversations' }).fill('quokka');
  const result = page.getByLabel('Search results').getByRole('button').first();
  await expect(result).toContainText('Tell me about animals');
  await expect(result.locator('mark')).toHaveText('quokkas');
  await result.click();
  await expect(lastReply(page)).toContainText('The answer involves quokkas.');
});

test('export writes the conversation as Markdown', async () => {
  await mock.setScript({ steps: [{ reply: { content: 'Exported text.' } }] });
  const { page } = app;
  await send(page, 'Export me');
  await expect(lastReply(page)).toContainText('Exported text.');
  await expect(page.getByRole('button', { name: 'Regenerate' })).toBeVisible();
  await page.getByRole('button', { name: 'Options for Export me' }).click();
  await page.getByRole('menuitem', { name: 'Export as Markdown' }).click();
  await expect
    .poll(() => {
      try {
        return readFileSync(join(saveDir, 'export.md'), 'utf8');
      } catch {
        return '';
      }
    })
    .toContain('Exported text.');
  const text = readFileSync(join(saveDir, 'export.md'), 'utf8');
  expect(text).toContain('# Export me');
  expect(text).toContain('**You**\n\nExport me');
});

test('a reply still being written survives a reload of the window', async () => {
  await mock.setScript({
    steps: [{ reply: { content: 'Survives reloads', stream: { chunk_chars: 4, hold: 'end' } } }],
  });
  const { page } = app;
  await send(page, 'Keep going');
  await expect(lastReply(page)).toContainText('Survives reloads');

  await page.reload();
  await page.waitForSelector('.hearth-app');
  await page
    .getByRole('navigation', { name: 'Conversations' })
    .getByRole('button', { name: 'Keep going', exact: true })
    .click();
  await expect(lastReply(page)).toContainText('Survives reloads');
  await expect(page.getByRole('button', { name: 'Stop' })).toBeVisible();
  await mock.release('end');
  await expect(lastReply(page).getByRole('button', { name: 'Regenerate' })).toBeVisible();
});

test('a server error shows on the reply, and Try again recovers', async () => {
  await mock.setScript({
    steps: [
      {
        reply: { fault: { http: { status: 500, body: { error: { message: 'model crashed' } } } } },
      },
      { reply: { content: 'Recovered fine.' } },
    ],
  });
  const { page } = app;
  await send(page, 'Hi');
  await expect(lastReply(page)).toContainText('model crashed');
  await lastReply(page).getByRole('button', { name: 'Try again' }).click();
  await expect(lastReply(page)).toContainText('Recovered fine.');
});

test('screenshots of a conversation in light and dark, for visual review', async () => {
  await mock.setScript({
    steps: [
      {
        reply: {
          content: RICH,
          reasoning: 'A table, some code and a formula.',
          usage: { prompt_tokens: 812, completion_tokens: 96 },
          timings: { predicted_per_second: 47.3 },
        },
      },
    ],
  });
  const { page } = app;
  const dir = join('test-results', 'screens');
  mkdirSync(dir, { recursive: true });
  await send(page, 'Summarize the benchmark and show the code');
  await expect(lastReply(page).getByRole('button', { name: 'Regenerate' })).toBeVisible();
  await lastReply(page)
    .getByRole('button', { name: /Thought for/ })
    .click();
  for (const theme of ['light', 'dark'] as const) {
    await page.evaluate((t) => document.documentElement.setAttribute('data-theme', t), theme);
    await page.waitForTimeout(150);
    await page.screenshot({ path: join(dir, `m2-chat-${theme}.png`) });
  }
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.waitForTimeout(400);
  await page.screenshot({ path: join(dir, 'm2-settings-dark.png') });
});

test.describe('without a model', () => {
  test('sending explains what to set up', async () => {
    const bare = await launchApp({ port: 9352 });
    try {
      const { page } = bare;
      await send(page, 'Hello?');
      await expect(page.getByRole('alert')).toContainText('Choose a model first');
      await expect(page.getByRole('button', { name: 'Open settings' })).toBeVisible();
    } finally {
      await bare.close();
    }
  });
});
