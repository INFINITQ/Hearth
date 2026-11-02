import { type Page, expect, test } from '@playwright/test';
import type Axe from 'axe-core';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { type RunningApp, launchApp } from './app';
import { type MockLlm, startMock } from './mock';

// Accessibility: axe-core checks every screen and state against WCAG 2.2 A/AA (and axe's best
// practices), in light and dark. Each test collects everything it finds and fails once with
// the whole list.

const AXE_SOURCE = readFileSync(
  createRequire(import.meta.url).resolve('axe-core/axe.min.js'),
  'utf8',
);

const RULES: Axe.RunOptions = {
  runOnly: {
    type: 'tag',
    values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'],
  },
  rules: {
    // WebView2 (Chromium 130 and later) puts scrolling regions such as wide code blocks in the
    // Tab order itself, which is what this rule asks for.
    'scrollable-region-focusable': { enabled: false },
  },
};

type Found = { rule: string; impact: string; help: string; nodes: string[] };

/** Runs axe on the page in light and dark; returns one line per problem. */
async function check(page: Page, where: string): Promise<string[]> {
  const lines: string[] = [];
  for (const theme of ['light', 'dark'] as const) {
    await page.evaluate((t) => {
      document.documentElement.setAttribute('data-theme', t);
      document.documentElement.style.colorScheme = t;
    }, theme);
    const found = await page.evaluate(async (options) => {
      const axe = (window as unknown as { axe: typeof Axe }).axe;
      const result = await axe.run(document, options);
      return result.violations.map((v): Found => ({
        rule: v.id,
        impact: v.impact ?? 'unknown',
        help: v.help,
        nodes: v.nodes.slice(0, 5).map((n) => {
          const why = n.failureSummary?.split('\n').slice(1, 2).join(' ').trim() ?? '';
          return `${n.html.slice(0, 120)} ${why}`;
        }),
      }));
    }, RULES);
    for (const f of found) {
      lines.push(
        `${where} (${theme}) ${f.rule} [${f.impact}] ${f.help}\n    ${f.nodes.join('\n    ')}`,
      );
    }
  }
  return lines;
}

/** Loads axe into the page (DevTools evaluation isn't bound by the app's content policy). */
async function prepare(page: Page) {
  // No transitions while checking: the hidden test window doesn't run them, so colors would be
  // read halfway through a theme change. (A rule added to the app's own stylesheet.)
  await page.evaluate(() => {
    const sheet = document.styleSheets[0]!;
    sheet.insertRule(
      '*, *::before, *::after { transition: none !important; }',
      sheet.cssRules.length,
    );
  });
  await page.evaluate(AXE_SOURCE);
  expect(await page.evaluate(() => typeof (window as unknown as { axe?: unknown }).axe)).toBe(
    'object',
  );
}

async function send(page: Page, text: string) {
  const box = page.getByRole('textbox', { name: 'Message' });
  await box.fill(text);
  await box.press('Enter');
}

let mock: MockLlm;
let app: RunningApp | undefined;
let project: string;

test.beforeAll(async () => {
  mock = await startMock();
});

test.afterAll(() => {
  mock.stop();
});

test.beforeEach(() => {
  project = mkdtempSync(join(tmpdir(), 'hearth-project-'));
  mkdirSync(join(project, 'src'));
  writeFileSync(join(project, 'src', 'calc.py'), 'def add(a, b):\n    return a - b\n');
});

test.afterEach(async () => {
  await app?.close();
  app = undefined;
  rmSync(project, { recursive: true, force: true });
});

test('before a model is chosen: get started, the Guide, Models, Settings and the palette', async () => {
  app = await launchApp();
  const { page } = app;
  await prepare(page);
  const problems: string[] = [];

  await expect(page.getByRole('region', { name: 'Get started' })).toBeVisible();
  problems.push(...(await check(page, 'chat, get started')));
  await page.getByRole('radio', { name: 'Code' }).click();
  problems.push(...(await check(page, 'code, no project')));
  await page.getByRole('button', { name: 'Guide' }).click();
  await expect(page.getByRole('heading', { name: 'Running models with Hearth' })).toBeVisible();
  problems.push(...(await check(page, 'guide')));
  await page.getByRole('button', { name: 'Models' }).click();
  problems.push(...(await check(page, 'models')));
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  problems.push(...(await check(page, 'settings')));

  await page.keyboard.press('Control+K');
  await expect(page.getByRole('dialog', { name: 'Commands' })).toBeVisible();
  problems.push(...(await check(page, 'command palette')));
  await page.keyboard.press('Escape');

  expect(problems, problems.join('\n')).toEqual([]);
});

test('a chat: markdown, code, math, thinking, and an error', async () => {
  await mock.setScript({
    steps: [
      {
        reply: {
          content: [
            '## Results',
            '',
            'See [the docs](https://example.com/docs) for more.',
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
          ].join('\n'),
          reasoning: 'Let me lay this out.',
        },
      },
      {
        reply: { fault: { http: { status: 500, body: { error: { message: 'model crashed' } } } } },
      },
    ],
  });
  app = await launchApp({ env: { HEARTH_TEST_LLM_URL: mock.url } });
  const { page } = app;
  await prepare(page);
  const problems: string[] = [];

  await send(page, 'Show me a summary');
  const reply = page.locator('[data-role="assistant"]').last();
  await expect(reply.locator('table')).toBeVisible();
  await reply.getByRole('button', { name: /Thought for/ }).click();
  problems.push(...(await check(page, 'chat reply')));

  await send(page, 'And again');
  await expect(page.locator('[data-role="assistant"]').last()).toContainText('model crashed');
  problems.push(...(await check(page, 'chat error')));

  await page.getByRole('button', { name: 'Chat settings' }).click();
  problems.push(...(await check(page, 'chat settings')));
  await page.keyboard.press('Escape');

  expect(problems, problems.join('\n')).toEqual([]);
});

test('a coding session: tools, a permission card, to-dos, a question, Changes and the terminal', async () => {
  const todos = [
    { content: 'Find the bug', status: 'completed' },
    { content: 'Fix the sign', status: 'in_progress' },
  ];
  await mock.setScript({
    steps: [
      { reply: { tool_calls: [{ name: 'todo_write', arguments: { todos } }] } },
      { reply: { tool_calls: [{ name: 'read_file', arguments: { path: 'src/calc.py' } }] } },
      {
        reply: {
          content: 'It subtracts. Fixing that.',
          tool_calls: [
            {
              name: 'edit_file',
              arguments: {
                path: 'src/calc.py',
                old_string: 'return a - b',
                new_string: 'return a + b',
              },
            },
          ],
        },
      },
      {
        reply: {
          tool_calls: [
            { name: 'ask_user', arguments: { question: 'Add a test?', options: ['Yes', 'No'] } },
          ],
        },
      },
      { reply: { content: 'Fixed `add`; no test added.' } },
    ],
  });
  app = await launchApp({
    env: { HEARTH_TEST_LLM_URL: mock.url, HEARTH_TEST_PICK_FOLDER: project },
  });
  const { page } = app;
  await prepare(page);
  const problems: string[] = [];

  await page.getByRole('radio', { name: 'Code' }).click();
  await page.getByRole('button', { name: 'Open folder…' }).click();
  await expect(page.getByRole('alertdialog')).toContainText('Trust the files in');
  problems.push(...(await check(page, 'trust dialog')));
  await page.getByRole('button', { name: 'Trust and open' }).click();
  await expect(page.getByRole('heading', { name: 'What should we work on?' })).toBeVisible();
  problems.push(...(await check(page, 'new session')));

  await page.getByRole('button', { name: /^Permission mode/ }).click();
  await page.getByRole('menuitem', { name: /^Ask/ }).click();
  await send(page, 'Fix add().');
  const prompt = page.getByRole('region', { name: 'Permission request' });
  await expect(prompt).toContainText('Allow this change?');
  problems.push(...(await check(page, 'permission request')));

  await prompt.getByRole('button', { name: 'Allow', exact: true }).click();
  const question = page.getByRole('region', { name: 'Question from the agent' });
  await expect(question).toContainText('Add a test?');
  problems.push(...(await check(page, 'question')));
  await question.getByRole('button', { name: 'No' }).click();
  await expect(page.getByText('no test added.', { exact: false })).toBeVisible();

  await page.getByRole('button', { name: /^Changes( \d+)?$/ }).click();
  await expect(page.getByRole('complementary', { name: 'Changes' })).toBeVisible();
  await page.getByRole('button', { name: 'Terminal', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Terminal' })).toBeVisible();
  problems.push(...(await check(page, 'session with Changes and the terminal')));

  expect(problems, problems.join('\n')).toEqual([]);
});
