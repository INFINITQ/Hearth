// Starts the testkit's mock model server (target/debug/hearth-mock-llm.exe, built by
// `cargo build -p hearth-testkit --bins`) and talks to its control endpoints.

import { spawn } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

export const MOCK_EXE = 'target/debug/hearth-mock-llm.exe';

export type MockLlm = {
  url: string;
  /** Replaces the script: each chat request consumes the next step. */
  setScript(script: { steps: object[]; model?: string }): Promise<void>;
  /** Lets replies held at `hold` finish. */
  release(hold: string): Promise<void>;
  requests(): Promise<Array<{ messages: Array<{ role: string; content: unknown }> }>>;
  stop(): void;
};

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

export async function startMock(): Promise<MockLlm> {
  const dir = mkdtempSync(join(tmpdir(), 'hearth-mock-'));
  const portFile = join(dir, 'port');
  const child = spawn(MOCK_EXE, ['--port', '0', '--port-file', portFile], { stdio: 'ignore' });
  let port = '';
  for (let attempt = 0; attempt < 200 && !port; attempt++) {
    try {
      port = readFileSync(portFile, 'utf8').trim();
    } catch {
      // not written yet
    }
    if (!port) await sleep(50);
  }
  if (!port) {
    child.kill();
    throw new Error(
      `${MOCK_EXE} never reported its port (did you run cargo build -p hearth-testkit --bins?)`,
    );
  }
  const url = `http://127.0.0.1:${port}`;
  const post = (path: string, body?: unknown) =>
    fetch(`${url}${path}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  return {
    url,
    setScript: async (script) => {
      await post('/__mock/reset');
      await post('/__mock/script', script);
    },
    release: async (hold) => {
      await post(`/__mock/release/${hold}`);
    },
    requests: async () => (await fetch(`${url}/__mock/requests`)).json(),
    stop: () => {
      child.kill();
      rmSync(dir, { recursive: true, force: true });
    },
  };
}
