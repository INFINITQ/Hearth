import { type Page, expect, test } from '@playwright/test';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { type RunningApp, launchApp, trackViolations } from './app';
import { type MockLlm, startMock } from './mock';

// The coding agent end to end: the real app, a mock model, and a throwaway project folder.

const CALC = 'def add(a, b):\n    return a - b\n';

let mock: MockLlm;
let app: RunningApp;
let project: string;

test.beforeAll(async () => {
  mock = await startMock();
});

test.afterAll(() => {
  mock.stop();
});

test.beforeEach(async () => {
  project = mkdtempSync(join(tmpdir(), 'hearth-project-'));
  mkdirSync(join(project, 'src'));
  writeFileSync(join(project, 'src', 'calc.py'), CALC);
  writeFileSync(join(project, 'AGENTS.md'), 'Run the tests with pytest.\n');
  app = await launchApp({
    env: { HEARTH_TEST_LLM_URL: mock.url, HEARTH_TEST_PICK_FOLDER: project },
  });
});

test.afterEach(async () => {
  await app.close();
  rmSync(project, { recursive: true, force: true });
});

const calc = () => readFileSync(join(project, 'src', 'calc.py'), 'utf8').replace(/\r\n/g, '\n');

const READ = { reply: { tool_calls: [{ name: 'read_file', arguments: { path: 'src/calc.py' } }] } };
const FIX = {
  reply: {
    content: 'It subtracts. Fixing that.',
    tool_calls: [
      {
        name: 'edit_file',
        arguments: { path: 'src/calc.py', old_string: 'return a - b', new_string: 'return a + b' },
      },
    ],
  },
};

async function openProject(page: Page) {
  await page.getByRole('radio', { name: 'Code' }).click();
  await page.getByRole('button', { name: 'Open folder…' }).click();
  await expect(page.getByRole('alertdialog')).toContainText('Trust the files in');
  await page.getByRole('button', { name: 'Trust and open' }).click();
  await expect(page.getByRole('heading', { name: 'What should we work on?' })).toBeVisible();
}

async function setMode(page: Page, label: string) {
  await page.getByRole('button', { name: /^Permission mode/ }).click();
  await page.getByRole('menuitem', { name: new RegExp(`^${label}`) }).click();
  await expect(page.getByRole('button', { name: `Permission mode: ${label}` })).toBeVisible();
}

async function send(page: Page, text: string) {
  const box = page.getByRole('textbox', { name: 'Message' });
  await box.fill(text);
  await box.press('Enter');
}

const tool = (page: Page, name: string) => page.locator(`[data-tool="${name}"]`).last();

test('the agent reads, edits and finishes, with the change shown as a diff', async () => {
  await mock.setScript({
    steps: [
      {
        expect: [{ system_contains: 'Run the tests with pytest.' }, { has_tool: 'edit_file' }],
        ...READ,
      },
      { expect: [{ last_tool_result_contains: '     2→    return a - b' }], ...FIX },
      { reply: { content: 'Fixed: `add` now returns `a + b`.' } },
    ],
  });
  const { page } = app;
  const violations = await trackViolations(page);
  await openProject(page);
  await send(page, 'add() subtracts. Fix it.');

  await expect(page.getByText('Fixed: add now returns a + b.')).toBeVisible();
  await expect(tool(page, 'read_file')).toHaveAttribute('data-status', 'ok');
  await expect(tool(page, 'read_file')).toContainText('Read src/calc.py');
  const edit = tool(page, 'edit_file');
  await expect(edit).toHaveAttribute('data-status', 'ok');
  await expect(edit.locator('[data-kind="add"]')).toContainText('return a + b');
  await expect(edit.locator('[data-kind="del"]')).toContainText('return a - b');
  expect(calc()).toBe('def add(a, b):\n    return a + b\n');
  await expect(
    page.getByRole('navigation', { name: 'Sessions' }).getByRole('button', {
      name: 'add() subtracts. Fix it.',
      exact: true,
    }),
  ).toBeVisible();
  expect(await violations()).toEqual([]);
});

test('Ask mode shows the diff first, and a denial with a note reaches the model', async () => {
  await mock.setScript({
    steps: [
      READ,
      FIX,
      {
        expect: [{ last_tool_result_contains: 'said: Use a named constant instead' }],
        reply: { content: 'Understood, I left it unchanged.' },
      },
    ],
  });
  const { page } = app;
  await openProject(page);
  await setMode(page, 'Ask');
  await send(page, 'Fix add().');

  const prompt = page.getByRole('region', { name: 'Permission request' });
  await expect(prompt).toContainText('Allow this change?');
  await expect(prompt.locator('[data-kind="add"]')).toContainText('return a + b');
  await expect(tool(page, 'edit_file')).toHaveAttribute('data-status', 'waiting');
  expect(calc()).toBe(CALC);

  await prompt
    .getByRole('textbox', { name: 'Tell the agent what to do instead' })
    .fill('Use a named constant instead');
  await prompt.getByRole('textbox', { name: 'Tell the agent what to do instead' }).press('Enter');
  await expect(page.getByText('Understood, I left it unchanged.')).toBeVisible();
  await expect(prompt).toBeHidden();
  await expect(tool(page, 'edit_file')).toHaveAttribute('data-status', 'denied');
  expect(calc()).toBe(CALC);
});

test('a waiting prompt survives a reload, and Allow applies the change', async () => {
  await mock.setScript({ steps: [READ, FIX, { reply: { content: 'Done.' } }] });
  const { page } = app;
  await openProject(page);
  await setMode(page, 'Ask');
  await send(page, 'Fix add() please');
  await expect(page.getByRole('region', { name: 'Permission request' })).toBeVisible();

  await page.reload();
  await page.waitForFunction(() => document.querySelector('.hearth-app') !== null);
  await page.getByRole('radio', { name: 'Code' }).click();
  const row = page
    .getByRole('navigation', { name: 'Sessions' })
    .getByRole('button', { name: 'Fix add() please', exact: true });
  await expect(page.getByRole('img', { name: 'Needs your answer' })).toBeVisible();
  await row.click();
  const prompt = page.getByRole('region', { name: 'Permission request' });
  await expect(prompt).toBeVisible();
  await prompt.getByRole('button', { name: 'Allow', exact: true }).click();
  await expect(page.getByText('Done.', { exact: true })).toBeVisible();
  expect(calc()).toBe('def add(a, b):\n    return a + b\n');
});

test('the to-do list and questions from the agent', async () => {
  const todos = [
    { content: 'Pick an indentation style', status: 'in_progress' },
    { content: 'Reformat calc.py', status: 'pending' },
  ];
  await mock.setScript({
    steps: [
      { reply: { tool_calls: [{ name: 'todo_write', arguments: { todos } }] } },
      {
        reply: {
          tool_calls: [
            {
              name: 'ask_user',
              arguments: { question: 'Tabs or spaces?', options: ['Tabs', 'Spaces'] },
            },
          ],
        },
      },
      {
        expect: [{ last_tool_result_contains: 'The user answered: Spaces' }],
        reply: { content: 'Spaces it is.' },
      },
    ],
  });
  const { page } = app;
  await openProject(page);
  await send(page, 'Set up formatting');

  const panel = page.getByRole('region', { name: 'To-do' });
  await expect(panel).toContainText('0 of 2 done');
  await expect(panel).toContainText('Pick an indentation style');
  const question = page.getByRole('region', { name: 'Question from the agent' });
  await expect(question).toContainText('Tabs or spaces?');
  await question.getByRole('button', { name: 'Spaces' }).click();
  await expect(page.getByText('Spaces it is.')).toBeVisible();
  await expect(question).toBeHidden();
});

test('screenshots of a coding session in light and dark, for visual review', async () => {
  const todos = [
    { content: 'Find the bug in add()', status: 'completed' },
    { content: 'Fix the sign', status: 'completed' },
    { content: 'Explain the change', status: 'in_progress' },
  ];
  await mock.setScript({
    steps: [
      {
        reply: {
          reasoning: 'The user says add() is wrong. I should read calc.py first.',
          content: "I'll start by reading the file.",
          tool_calls: [{ name: 'read_file', arguments: { path: 'src/calc.py' } }],
        },
      },
      { reply: { tool_calls: [{ name: 'todo_write', arguments: { todos } }] } },
      FIX,
      {
        reply: {
          content:
            'Fixed `add` in `src/calc.py`: it returned `a - b`, so every sum came out wrong. It now returns `a + b`.',
          usage: { prompt_tokens: 2100, completion_tokens: 38, total_tokens: 2138 },
          timings: { predicted_per_second: 41.7 },
        },
      },
    ],
  });
  const { page } = app;
  const dir = join('test-results', 'screens');
  mkdirSync(dir, { recursive: true });
  await openProject(page);
  await send(page, 'add() in calc.py gives wrong results. Find and fix the bug.');
  await expect(page.getByText(/It now returns/)).toBeVisible();
  for (const theme of ['light', 'dark'] as const) {
    await page.evaluate((t) => document.documentElement.setAttribute('data-theme', t), theme);
    await page.waitForTimeout(200);
    await page.screenshot({ path: join(dir, `m3-code-${theme}.png`) });
  }
});

test.describe('commands, changes, undo and steering', () => {
  const runCommand = (command: string) => ({
    reply: { tool_calls: [{ name: 'run_command', arguments: { command } }] },
  });

  test('a command runs and its output shows in the transcript', async () => {
    await mock.setScript({
      steps: [
        runCommand('echo e2e-from-bash && echo second-line'),
        {
          expect: [{ last_tool_result_contains: 'e2e-from-bash' }],
          reply: { content: 'It printed two lines.' },
        },
      ],
    });
    const { page } = app;
    await openProject(page);
    await setMode(page, 'Full auto');
    await send(page, 'Run the echo.');
    await expect(page.getByText('It printed two lines.')).toBeVisible();
    const row = tool(page, 'run_command');
    await expect(row).toHaveAttribute('data-status', 'ok');
    await row.locator('button[aria-expanded]').click();
    await expect(row.getByLabel('Command output')).toContainText('second-line');
  });

  test('commands ask first in Ask mode, and Always allow is remembered', async () => {
    await mock.setScript({
      steps: [runCommand('echo one'), runCommand('echo two'), { reply: { content: 'Both done.' } }],
    });
    const { page } = app;
    await openProject(page);
    await setMode(page, 'Ask');
    await send(page, 'Echo twice.');
    const prompt = page.getByRole('region', { name: 'Permission request' });
    await expect(prompt).toContainText('Allow this command?');
    await expect(prompt.getByLabel('Command')).toContainText('echo one');
    await prompt.getByRole('button', { name: 'Always allow `echo` here' }).click();
    await expect(page.getByText('Both done.')).toBeVisible();
    await expect(page.locator('[data-tool="run_command"][data-status="ok"]')).toHaveCount(2);
  });

  test('the Changes panel lists what changed and can revert a file', async () => {
    await mock.setScript({ steps: [READ, FIX, { reply: { content: 'Fixed.' } }] });
    const { page } = app;
    await openProject(page);
    await send(page, 'Fix add().');
    await expect(page.getByText('Fixed.', { exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Changes', exact: true }).click();
    const panel = page.getByRole('complementary', { name: 'Changes' });
    await expect(panel.locator('[data-change="modified"]')).toContainText('calc.py');
    await panel.getByRole('button', { name: 'Revert src/calc.py' }).click();
    await page.getByRole('alertdialog').getByRole('button', { name: 'Revert' }).click();
    await expect(panel).toContainText("The agent hasn't changed any files yet.");
    expect(calc()).toBe(CALC);
  });

  test('undoing a task puts its files back', async () => {
    await mock.setScript({ steps: [READ, FIX, { reply: { content: 'Fixed.' } }] });
    const { page } = app;
    await openProject(page);
    await send(page, 'Fix add() now');
    await expect(page.getByText('Fixed.', { exact: true })).toBeVisible();
    expect(calc()).toBe('def add(a, b):\n    return a + b\n');
    const task = page.locator('[data-role="user"]').first();
    await task.hover();
    await task.getByRole('button', { name: 'Undo file changes from here' }).click();
    await page.getByRole('alertdialog').getByRole('button', { name: 'Undo changes' }).click();
    await expect(page.getByRole('status')).toContainText('Restored 1 file');
    expect(calc()).toBe(CALC);
  });

  test('a message sent while the agent works joins the next step', async () => {
    await mock.setScript({
      steps: [
        {
          reply: {
            tool_calls: [{ name: 'glob', arguments: { pattern: '*.py' } }],
            stream: { hold: 'first' },
          },
        },
        {
          expect: [{ last_user_contains: 'Also check the README' }],
          reply: { content: 'Checked both.' },
        },
      ],
    });
    const { page } = app;
    await openProject(page);
    await send(page, 'Find the Python files.');
    await expect(page.getByRole('button', { name: 'Stop' })).toBeVisible();
    await send(page, 'Also check the README');
    await expect(page.getByRole('list', { name: 'Waiting to be sent' })).toContainText(
      'Also check the README',
    );
    await mock.release('first');
    await expect(page.getByText('Checked both.')).toBeVisible();
    await expect(page.locator('[data-role="user"]')).toHaveCount(2);
    await expect(page.getByRole('list', { name: 'Waiting to be sent' })).toBeHidden();
  });

  test('screenshots of commands and the Changes panel, for visual review', async () => {
    await mock.setScript({
      steps: [
        READ,
        FIX,
        runCommand('echo "test_add PASSED" && echo "1 passed in 0.02s"'),
        {
          reply: {
            content: 'Fixed `add` and the test passes now.',
            usage: { prompt_tokens: 2600, completion_tokens: 21, total_tokens: 2621 },
            timings: { predicted_per_second: 44.2 },
          },
        },
      ],
    });
    const { page } = app;
    const dir = join('test-results', 'screens');
    mkdirSync(dir, { recursive: true });
    await openProject(page);
    await setMode(page, 'Full auto');
    await send(page, 'Fix add() and run the tests.');
    await expect(page.getByText(/the test passes now/)).toBeVisible();
    await tool(page, 'run_command').locator('button[aria-expanded]').click();
    await page.getByRole('button', { name: 'Changes', exact: true }).click();
    await page
      .getByRole('complementary', { name: 'Changes' })
      .locator('[data-change] button[aria-expanded]')
      .first()
      .click();
    for (const theme of ['light', 'dark'] as const) {
      await page.evaluate((t) => document.documentElement.setAttribute('data-theme', t), theme);
      await page.waitForTimeout(200);
      await page.screenshot({ path: join(dir, `m4-code-${theme}.png`) });
    }
  });
});
