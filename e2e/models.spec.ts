import { type Page, expect, test } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { type RunningApp, launchApp, trackViolations } from './app';

// Hearth's own model runtime end to end: the real app, the testkit's fake llama-server
// (target/debug/fake-llama-server.exe) and GGUF headers written here. Nothing is downloaded.

const FAKE_SERVER = resolve('target/debug/fake-llama-server.exe');

/** A GGUF v3 file with this metadata and no tensors. */
function gguf(entries: [string, string | number | boolean][]): Buffer {
  const u32 = (n: number) => {
    const b = Buffer.alloc(4);
    b.writeUInt32LE(n);
    return b;
  };
  const u64 = (n: bigint) => {
    const b = Buffer.alloc(8);
    b.writeBigUInt64LE(n);
    return b;
  };
  const str = (s: string) => Buffer.concat([u64(BigInt(Buffer.byteLength(s))), Buffer.from(s)]);
  const parts = [Buffer.from('GGUF'), u32(3), u64(0n), u64(BigInt(entries.length))];
  for (const [key, value] of entries) {
    parts.push(str(key));
    if (typeof value === 'string') parts.push(u32(8), str(value));
    else if (typeof value === 'number') parts.push(u32(4), u32(value));
    else parts.push(u32(7), Buffer.from([value ? 1 : 0]));
  }
  return Buffer.concat(parts);
}

let models: string;
let app: RunningApp;

function writeModels() {
  models = mkdtempSync(join(tmpdir(), 'hearth-models-'));
  mkdirSync(join(models, 'qwen'));
  mkdirSync(join(models, 'gpt-oss'));
  writeFileSync(
    join(models, 'qwen', 'Qwen3.6-35B-A3B-UD-IQ4_XS.gguf'),
    gguf([
      ['general.architecture', 'qwen3moe'],
      ['general.name', 'Qwen3.6 35B A3B'],
      ['general.size_label', '35B-A3B'],
      ['general.file_type', 30],
      ['qwen3moe.context_length', 262144],
      ['qwen3moe.expert_count', 128],
      ['tokenizer.chat_template', '{% if tools %}{% endif %}{% if enable_thinking %}{% endif %}'],
    ]),
  );
  writeFileSync(
    join(models, 'qwen', 'mmproj-F16.gguf'),
    gguf([
      ['general.architecture', 'clip'],
      ['general.file_type', 1],
      ['clip.has_vision_encoder', true],
    ]),
  );
  writeFileSync(
    join(models, 'gpt-oss', 'gpt-oss-20b-MXFP4.gguf'),
    gguf([
      ['general.architecture', 'gpt-oss'],
      ['general.name', 'gpt-oss-20b'],
      ['gpt-oss.context_length', 131072],
    ]),
  );
}

async function start(extraEnv: Record<string, string> = {}, serverPath = FAKE_SERVER) {
  app = await launchApp({
    env: {
      HEARTH_TEST_SETTINGS: JSON.stringify({
        runtime: { llamaServerPath: serverPath, modelFolders: [models] },
      }),
      FAKE_LLAMA_LOAD_MS: '600',
      ...extraEnv,
    },
  });
}

test.beforeEach(() => writeModels());

test.afterEach(async () => {
  await app.close();
  rmSync(models, { recursive: true, force: true });
});

const qwen = (page: Page) => page.getByRole('article', { name: 'Qwen3.6 35B A3B' });

async function openModels(page: Page) {
  await page.getByRole('button', { name: 'Models', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Models', exact: true })).toBeVisible();
}

test('the Models page finds llama-server and the models, and loads one', async () => {
  await start();
  const { page } = app;
  const violations = await trackViolations(page);
  await openModels(page);
  await expect(page.getByText('build 9999 (fa4e11a)')).toBeVisible();
  await expect(page.getByText('NVIDIA GeForce RTX 5070 · 12 GB', { exact: false })).toBeVisible();
  await expect(qwen(page)).toContainText('UD-IQ4_XS');
  await expect(qwen(page)).toContainText('256K context');
  await expect(qwen(page)).toContainText('vision');
  await expect(page.getByRole('article', { name: 'gpt-oss-20b' })).toBeVisible();

  await qwen(page).getByRole('button', { name: 'Load' }).click();
  await expect(qwen(page).locator('[data-state="ready"]')).toBeVisible();
  await qwen(page).getByRole('button', { name: 'Coding model' }).click();
  await expect(qwen(page).getByRole('button', { name: /Coding model/ })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  await qwen(page).getByRole('button', { name: 'Unload' }).click();
  await expect(qwen(page).locator('[data-state="idle"]')).toBeVisible();
  expect(await violations()).toEqual([]);
});

test('a chat with a model on this PC loads it first, then answers', async () => {
  await start({ FAKE_LLAMA_LOAD_MS: '1500' });
  const { page } = app;
  await openModels(page);
  await qwen(page).getByRole('button', { name: 'Chat model' }).click();
  await page.getByRole('radio', { name: 'Chat' }).click();
  await expect(page.getByRole('button', { name: /^Model: / })).toContainText('Qwen3.6 35B A3B');
  const box = page.getByRole('textbox', { name: 'Message' });
  await box.fill('Hi there');
  await box.press('Enter');
  await expect(page.getByText(/Loading Qwen3\.6 35B A3B…/)).toBeVisible();
  await expect(page.getByText(/^Hello from m-[0-9a-f]+\.$/)).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('[data-role="assistant"]').last()).toContainText('Qwen3.6 35B A3B');
});

test('a model that runs out of memory says why, with its log', async () => {
  await start({ FAKE_LLAMA_FAIL: 'oom' });
  const { page } = app;
  await openModels(page);
  await qwen(page).getByRole('button', { name: 'Load' }).click();
  const alert = qwen(page).getByRole('alert');
  await expect(alert).toContainText('Not enough GPU or system memory');
  await alert.getByRole('button', { name: 'Show the log' }).click();
  await expect(alert).toContainText('cudaMalloc failed: out of memory');
});

test('without llama-server the page explains the setup, and choosing it works', async () => {
  await start({ HEARTH_TEST_PICK_FILE: FAKE_SERVER }, 'C:\\nowhere\\llama-server.exe');
  const { page } = app;
  await openModels(page);
  await expect(page.getByRole('heading', { name: 'Set up llama.cpp' })).toBeVisible();
  await expect(page.getByText(/doesn't exist/)).toBeVisible();
  await page.getByRole('button', { name: 'Choose llama-server.exe…' }).click();
  await expect(page.getByText('build 9999 (fa4e11a)')).toBeVisible();
});

test('screenshots of the Models page, for visual review', async () => {
  await start();
  const { page } = app;
  const dir = join('test-results', 'screens');
  mkdirSync(dir, { recursive: true });
  await openModels(page);
  await qwen(page).getByRole('button', { name: 'Load' }).click();
  await expect(qwen(page).locator('[data-state="ready"]')).toBeVisible();
  await qwen(page).getByRole('button', { name: 'Chat model' }).click();
  await qwen(page).getByRole('button', { name: 'Settings' }).click();
  for (const theme of ['light', 'dark'] as const) {
    await page.evaluate((t) => document.documentElement.setAttribute('data-theme', t), theme);
    await page.waitForTimeout(200);
    await page.screenshot({ path: join(dir, `m5-models-${theme}.png`) });
  }
});
