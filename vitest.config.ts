import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['apps/*/src/**/*.test.{ts,tsx}', 'config/**/*.test.ts'],
    environment: 'jsdom',
    restoreMocks: true,
  },
});
