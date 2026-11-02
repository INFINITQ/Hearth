import { defineConfig } from '@playwright/test';

// End-to-end tests drive the real debug build (`pnpm build:e2e`) over WebView2's debugging
// port, with the window hidden (HEARTH_BACKGROUND=1) and a throwaway data folder. No browser is
// downloaded. One app at a time.
export default defineConfig({
  testDir: 'e2e',
  testMatch: '**/*.spec.ts',
  workers: 1,
  fullyParallel: false,
  timeout: 60_000,
  expect: { timeout: 10_000 },
  reporter: [['list']],
  outputDir: 'test-results/artifacts',
});
