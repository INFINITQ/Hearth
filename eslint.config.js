// @ts-check
import js from '@eslint/js';
import { defineConfig, globalIgnores } from 'eslint/config';
import reactHooks from 'eslint-plugin-react-hooks';
import globals from 'globals';
import tseslint from 'typescript-eslint';

// The UI never opens network connections itself: llama-server, web search and MCP are all
// reached from Rust, which applies Hearth's rules. See docs/adr/0003.
const NO_NETWORK =
  'The UI must not use the network directly; call the Rust backend instead (docs/adr/0003).';

const networkGlobals = [
  'fetch',
  'XMLHttpRequest',
  'WebSocket',
  'EventSource',
  'RTCPeerConnection',
  'WebTransport',
].map((name) => ({ name, message: NO_NETWORK }));

const networkProperties = [
  { object: 'navigator', property: 'sendBeacon', message: NO_NETWORK },
  { object: 'window', property: 'fetch', message: NO_NETWORK },
  { object: 'globalThis', property: 'fetch', message: NO_NETWORK },
  { object: 'window', property: 'open', message: 'Open links through backend().openLink().' },
];

export default defineConfig([
  globalIgnores([
    '**/node_modules/',
    '**/dist/',
    '**/target/',
    '**/src-tauri/gen/',
    'playwright-report/',
    'test-results/',
  ]),
  js.configs.recommended,
  tseslint.configs.recommended,
  {
    files: ['apps/*/src/**/*.{ts,tsx}'],
    languageOptions: { globals: globals.browser },
    plugins: { 'react-hooks': reactHooks },
    rules: {
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'warn',
      'no-restricted-globals': ['error', ...networkGlobals],
      'no-restricted-properties': ['error', ...networkProperties],
    },
  },
  {
    // Tests, tooling and config files run in Node or drive the app from outside.
    files: [
      '**/*.test.{ts,tsx}',
      'e2e/**',
      'tools/**',
      '*.config.{js,ts}',
      'apps/*/*.config.{js,ts}',
    ],
    languageOptions: { globals: { ...globals.node, ...globals.browser } },
    rules: { 'no-restricted-globals': 'off', 'no-restricted-properties': 'off' },
  },
]);
