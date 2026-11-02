import { type Page, expect, test } from '@playwright/test';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
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

test('a long chat is summarized to fit the window, with a divider and the context meter', async () => {
  // About 4,000 characters: more than the 35% of a 2,048-token window that recent messages keep.
  const plan = Array.from(
    { length: 40 },
    (_, i) =>
      `- Day ${i + 1}: a temple in the morning, a long walk by the river after lunch, dinner near Gion.`,
  ).join('\n');
  await mock.setScript({
    n_ctx: 2048,
    steps: [
      { reply: { content: plan, usage: { prompt_tokens: 600, completion_tokens: 1100 } } },
      // The summary of the first exchange (not streamed).
      {
        reply: {
          content:
            '- The user is planning 40 days in Kyoto.\n- Every day: a temple, a river walk, dinner near Gion.',
        },
      },
      {
        reply: {
          content: 'Day 3 is the quietest one.',
          usage: { prompt_tokens: 420, completion_tokens: 60 },
        },
      },
    ],
  });
  const { page } = app;
  const dir = join('test-results', 'screens');
  mkdirSync(dir, { recursive: true });
  await send(page, 'Plan 40 days in Kyoto');
  await expect(lastReply(page)).toContainText('Day 40');
  const meter = page.getByRole('meter', { name: 'Context used' });
  await expect(meter).toHaveText('83%');

  await send(page, 'Which day is the quietest?');
  await expect(lastReply(page)).toContainText('Day 3 is the quietest one.');
  const divider = page.locator('[data-role="summary"]');
  await expect(divider).toContainText('Earlier messages were summarized to make room');
  await expect(meter).toHaveText('23%');

  // The reply was asked with the summary standing in for the long plan.
  const requests = await mock.requests();
  expect(requests).toHaveLength(3);
  const asked = requests[2]!.messages;
  expect(String(asked[0]!.content)).toContain('<summary_of_earlier_conversation>');
  expect(JSON.stringify(asked)).not.toContain('Day 40');
  // The plan itself stays in the transcript.
  await expect(page.getByText('Day 40', { exact: false })).toBeVisible();

  await divider.getByRole('button').click();
  await expect(divider).toContainText('planning 40 days in Kyoto');
  for (const theme of ['light', 'dark'] as const) {
    await page.evaluate((t) => document.documentElement.setAttribute('data-theme', t), theme);
    await page.waitForTimeout(150);
    await page.screenshot({ path: join(dir, `m6-summary-${theme}.png`) });
  }
});

const NEWS = `<!doctype html><html><head><title>Hearth 1.0</title></head><body>
<nav>Home · Blog</nav><article><h1>Hearth 1.0</h1>
<p>Hearth 1.0 ships on 14 November with web search, terminals and MCP servers. It runs local
models with llama.cpp and keeps everything on your PC.</p>
<p>The release also brings a context meter and summaries of long conversations, so a chat can go
on for as long as you like.</p></article></body></html>`;

test('web search: set up in Settings, then a reply searches, reads and cites its sources', async () => {
  const page = `${mock.url}/pages/news`;
  await mock.setScript({
    search: [{ title: 'Hearth 1.0 is out', url: page, content: 'The local AI app ships soon.' }],
    pages: { news: NEWS },
    steps: [
      {
        expect: [{ has_tool: 'web_search' }],
        reply: { tool_calls: [{ name: 'web_search', arguments: { query: 'hearth 1.0 release' } }] },
      },
      {
        expect: [{ last_tool_result_contains: 'Hearth 1.0 is out' }],
        reply: { tool_calls: [{ name: 'web_fetch', arguments: { url: page } }] },
      },
      {
        expect: [{ last_tool_result_contains: 'ships on 14 November' }],
        reply: { content: `Hearth 1.0 ships on **14 November** ([the announcement](${page})).` },
      },
    ],
  });
  const { page: ui } = app;

  // Settings → Web: a SearXNG server (the mock), local pages allowed, and a test search.
  await ui.getByRole('button', { name: 'Settings', exact: true }).click();
  await ui.getByLabel('Search with').selectOption('searxng');
  const address = ui.getByLabel('SearXNG address');
  await address.fill(mock.url);
  await address.blur();
  await ui.getByLabel('Read pages on this PC and your network').check();
  await ui.getByRole('button', { name: 'Test search' }).click();
  await expect(ui.getByText('It works: 1 result.')).toBeVisible();

  await ui.getByRole('radio', { name: 'Chat' }).click();
  const web = ui.getByRole('button', { name: 'Search the web' });
  await web.click();
  await expect(web).toHaveAttribute('aria-pressed', 'true');
  await send(ui, 'When does Hearth 1.0 ship?');
  await expect(lastReply(ui)).toContainText('ships on 14 November');

  const searched = ui.locator('[data-web="search"]');
  await expect(searched).toContainText('Searched “hearth 1.0 release”');
  await expect(searched).toContainText('1 result');
  await expect(ui.locator('[data-web="read"]')).toContainText('Read Hearth 1.0');
  const sources = ui.getByRole('list', { name: 'Sources' });
  await expect(sources).toContainText('Hearth 1.0');
  await searched.getByRole('button').first().click();
  await expect(searched).toContainText('The local AI app ships soon.');

  const dir = join('test-results', 'screens');
  mkdirSync(dir, { recursive: true });
  for (const theme of ['light', 'dark'] as const) {
    await ui.evaluate((t) => document.documentElement.setAttribute('data-theme', t), theme);
    await ui.waitForTimeout(150);
    await ui.screenshot({ path: join(dir, `m8-web-chat-${theme}.png`) });
  }
  await ui.getByRole('button', { name: 'Settings', exact: true }).click();
  await ui.getByRole('heading', { name: 'Settings' }).waitFor();
  await ui.getByText('Read pages on this PC and your network').scrollIntoViewIfNeeded();
  await ui.waitForTimeout(150);
  await ui.screenshot({ path: join(dir, 'm8-web-settings-dark.png') });
});

/** A one-page PDF saying `text` (Helvetica, with a proper cross-reference table). */
function tinyPdf(text: string): Buffer {
  const stream = `BT /F1 24 Tf 36 72 Td (${text}) Tj ET`;
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 144] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>',
    `<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
  ];
  let pdf = '%PDF-1.4\n';
  const offsets: number[] = [];
  objects.forEach((body, index) => {
    offsets.push(pdf.length);
    pdf += `${index + 1} 0 obj\n${body}\nendobj\n`;
  });
  const xref = pdf.length;
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  pdf += offsets.map((offset) => `${String(offset).padStart(10, '0')} 00000 n \n`).join('');
  pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(pdf, 'latin1');
}

/** A 1×1 PNG. */
const DOT = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==',
  'base64',
);

test('attached files, a PDF and an image reach a model that can see', async () => {
  await mock.setScript({
    vision: true,
    steps: [
      {
        expect: [
          { last_user_contains: '<attachment name="groceries.md">' },
          { last_user_contains: 'buy oranges' },
          { last_user_contains: 'Hello from a PDF' },
          { image_count: 1 },
        ],
        reply: { content: 'A shopping list, a one-line PDF and a single dot.' },
      },
    ],
  });
  const files = join(saveDir, 'files');
  mkdirSync(files, { recursive: true });
  writeFileSync(join(files, 'groceries.md'), '# Groceries\n\n- buy oranges\n');
  writeFileSync(join(files, 'note.pdf'), tinyPdf('Hello from a PDF'));
  writeFileSync(join(files, 'dot.png'), DOT);
  const { page } = app;
  await page
    .getByLabel('Files to attach')
    .setInputFiles(['groceries.md', 'note.pdf', 'dot.png'].map((name) => join(files, name)));
  const draft = page.getByLabel('Files to send');
  await expect(draft.locator('[data-file="note.pdf"]')).toContainText('PDF');
  await expect(draft.locator('img[alt="dot.png"]')).toBeVisible();
  await send(page, 'What are these?');
  await expect(lastReply(page)).toContainText('A shopping list, a one-line PDF and a single dot.');

  const sent = page.locator('[data-role="user"]').last();
  await expect(sent.locator('[data-file="groceries.md"]')).toBeVisible();
  await expect(sent.locator('[data-file="note.pdf"]')).toContainText('PDF');
  await expect(sent.locator('img[data-image="dot.png"]')).toBeVisible();
  await expect(page.getByLabel('Files to send')).toHaveCount(0);

  const dir = join('test-results', 'screens');
  mkdirSync(dir, { recursive: true });
  for (const theme of ['light', 'dark'] as const) {
    await page.evaluate((t) => document.documentElement.setAttribute('data-theme', t), theme);
    await page.waitForTimeout(150);
    await page.screenshot({ path: join(dir, `m10-attachments-${theme}.png`) });
  }
});

test('a model that cannot see images gets the text, and the chat says why', async () => {
  await mock.setScript({
    steps: [
      {
        expect: [{ image_count: 0 }, { last_user_contains: "can't see images" }],
        reply: { content: 'I only got your words.' },
      },
    ],
  });
  const files = join(saveDir, 'files');
  mkdirSync(files, { recursive: true });
  writeFileSync(join(files, 'dot.png'), DOT);
  const { page } = app;
  await page.getByLabel('Files to attach').setInputFiles(join(files, 'dot.png'));
  await expect(page.getByLabel('Files to send').locator('img[alt="dot.png"]')).toBeVisible();
  await send(page, 'Look at this.');
  await expect(lastReply(page)).toContainText('I only got your words.');
  await expect(page.getByRole('status')).toContainText("can't see images");
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
