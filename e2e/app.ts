// Launches a Hearth debug build hidden, with WebView2's debugging port open, and connects
// Playwright to its page (Folio's harness, adapted). Debug builds only: release builds drop
// every WEBVIEW2_* variable. Each launch gets a fresh data folder (HEARTH_TEST_DATA_DIR), so
// tests never touch the installed app's settings or conversations.

import { type Browser, type Page, chromium } from '@playwright/test';
import { type ChildProcess, spawn } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

export const EXE = 'target/debug/hearth.exe';

export type RunningApp = {
  page: Page;
  /** The app's process (WebView2's processes are its children). */
  pid: number;
  /** This launch's data folder (settings.json, hearth.db, …). */
  dataDir: string;
  /** Stops the app and deletes its data folder; fails if the page threw an uncaught error. */
  close(): Promise<void>;
};

/** Deletes a test data folder; WebView2 lets go of its profile a moment after the app stops. */
function removeDataDir(dir: string) {
  try {
    rmSync(dir, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
  } catch {
    // Still in use: it's in the temp folder, which Windows cleans up.
  }
}

export async function launchApp(
  options: { exe?: string; port?: number; env?: Record<string, string> } = {},
): Promise<RunningApp> {
  const exe = options.exe ?? EXE;
  const port = options.port ?? 9351;
  const dataDir = mkdtempSync(join(tmpdir(), 'hearth-data-'));
  const env = {
    ...process.env,
    ...options.env,
    HEARTH_BACKGROUND: '1',
    HEARTH_TEST_DATA_DIR: dataDir,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
  };
  const child: ChildProcess = spawn(exe, [], { env, stdio: 'ignore' });
  let browser: Browser | undefined;
  for (let attempt = 0; attempt < 80 && !browser; attempt++) {
    try {
      browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
  }
  if (!browser) {
    child.kill();
    removeDataDir(dataDir);
    throw new Error(`${exe} never opened its debugging port`);
  }
  const page = browser.contexts()[0]!.pages()[0]!;
  // Uncaught errors fail the test (see close): code that works in a plain browser can still
  // break in the app, for example under Tauri's frozen prototypes.
  const errors: string[] = [];
  page.on('pageerror', (error) =>
    errors.push((error.stack ?? String(error)).split('\n').slice(0, 8).join(' | ')),
  );
  const started = () =>
    page.waitForFunction(() => document.querySelector('.hearth-app') !== null, null, {
      timeout: 15_000,
    });
  try {
    await started();
  } catch {
    // It went wrong before we were watching: load it again to see why.
    await page.reload();
    try {
      await started();
    } catch {
      child.kill();
      removeDataDir(dataDir);
      throw new Error(`${exe} didn't start: ${errors.join(' || ') || 'no error was reported'}`);
    }
  }
  return {
    page,
    pid: child.pid ?? 0,
    dataDir,
    close: async () => {
      child.kill();
      await new Promise((resolve) => setTimeout(resolve, 300));
      removeDataDir(dataDir);
      if (errors.length) throw new Error(`The app threw uncaught errors:\n${errors.join('\n')}`);
    },
  };
}

/** Records content-policy violations from now on. */
export async function trackViolations(page: Page): Promise<() => Promise<string[]>> {
  await page.evaluate(() => {
    const target = window as unknown as { __violations: string[] };
    target.__violations = [];
    document.addEventListener('securitypolicyviolation', (e) =>
      target.__violations.push(`${e.effectiveDirective} ${e.blockedURI || '(inline)'}`),
    );
  });
  return () => page.evaluate(() => (window as unknown as { __violations: string[] }).__violations);
}
