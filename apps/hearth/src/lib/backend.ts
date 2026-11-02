import { Channel, invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { fakeChat } from './fakeChat';
import {
  type Boot,
  type ChatAction,
  type Connection,
  type ConnectionInput,
  DEFAULT_SETTINGS,
  type Mode,
  type RunEnvelope,
  type RunInfo,
  type SearchHit,
  type Session,
  type SessionPatch,
  type SessionSummary,
  type SessionView,
  type Settings,
  type StartedRun,
  type SystemInfo,
} from './types';

export type CaptionState = 'enter' | 'leave' | 'press' | 'release';

/** Window controls for the custom title bar. */
export interface WindowControls {
  minimize(): void;
  toggleMaximize(): void;
  close(): void;
  isMaximized(): Promise<boolean>;
  onResized(listener: () => void): () => void;
  /** Tells Rust where the HTML maximize button is, for Windows 11's Snap Layouts. */
  reportMaximizeRect(rect: { right: number; top: number; width: number; height: number }): void;
  /** Hover and press state from the native Snap Layouts overlay. */
  onCaption(listener: (state: CaptionState) => void): () => void;
}

/** A JSON merge patch for the settings: `null` resets a value to its default. */
export type SettingsPatch = { [K in keyof Settings]?: Partial<Settings[K]> | Settings[K] | null };

/** Conversations, runs and connections. */
export interface ChatBackend {
  sessionsList(mode: Mode): Promise<SessionSummary[]>;
  sessionCreate(mode: Mode): Promise<Session>;
  sessionGet(id: string): Promise<SessionView>;
  sessionUpdate(id: string, patch: SessionPatch): Promise<Session>;
  sessionDelete(id: string): Promise<void>;
  branchSelect(sessionId: string, messageId: string): Promise<SessionView>;
  search(query: string, mode?: Mode): Promise<SearchHit[]>;
  sessionExport(id: string, format: 'md' | 'json'): Promise<string | null>;
  connectionsList(): Promise<Connection[]>;
  connectionSave(input: ConnectionInput): Promise<Connection>;
  connectionDelete(id: string): Promise<void>;
  connectionModels(id: string): Promise<string[]>;
  connectionTest(baseUrl: string, apiKey?: string | null): Promise<string[]>;
  runStart(
    sessionId: string,
    action: ChatAction,
    onEvent: (e: RunEnvelope) => void,
  ): Promise<StartedRun>;
  runAttach(runId: string, onEvent: (e: RunEnvelope) => void): Promise<boolean>;
  runCancel(runId: string): Promise<boolean>;
  runsActive(): Promise<RunInfo[]>;
  onSessionsChanged(listener: () => void): () => void;
  onRunState(listener: (info: RunInfo) => void): () => void;
  openLink(url: string): Promise<void>;
}

/**
 * Everything the UI asks of the outside world. The app uses Tauri; unit tests and a plain
 * browser (`pnpm dev:web`) use an in-memory fake.
 */
export interface Backend extends ChatBackend {
  readonly kind: 'tauri' | 'fake';
  boot(): Boot;
  appReady(): Promise<void>;
  systemInfo(): Promise<SystemInfo>;
  settingsGet(): Promise<Settings>;
  settingsSet(patch: SettingsPatch): Promise<Settings>;
  onSettings(listener: (settings: Settings) => void): () => void;
  setWindowDark(dark: boolean): Promise<void>;
  window: WindowControls;
}

declare global {
  interface Window {
    __HEARTH_BOOT__?: Boot;
    __TAURI_INTERNALS__?: unknown;
  }
}

function systemDark(): boolean {
  return typeof matchMedia === 'function' && matchMedia('(prefers-color-scheme: dark)').matches;
}

/** Turns an async unlisten into a plain function that works even before `listen` resolves. */
function unsubscriber(pending: Promise<() => void>): () => void {
  let stopped = false;
  let stop: (() => void) | undefined;
  void pending.then((fn) => {
    if (stopped) fn();
    else stop = fn;
  });
  return () => {
    stopped = true;
    stop?.();
  };
}

function subscribe<T>(event: string, listener: (payload: T) => void): () => void {
  return unsubscriber(listen<T>(event, (e) => listener(e.payload)));
}

function channel(onEvent: (e: RunEnvelope) => void): Channel<RunEnvelope> {
  const next = new Channel<RunEnvelope>();
  next.onmessage = onEvent;
  return next;
}

export function tauriBackend(): Backend {
  const win = getCurrentWindow();
  return {
    kind: 'tauri',
    boot: () => window.__HEARTH_BOOT__ ?? { settings: DEFAULT_SETTINGS, systemDark: systemDark() },
    appReady: () => invoke('app_ready'),
    systemInfo: () => invoke<SystemInfo>('system_info'),
    settingsGet: () => invoke<Settings>('settings_get'),
    settingsSet: (patch) => invoke<Settings>('settings_set', { patch }),
    onSettings: (listener) => subscribe<Settings>('hearth://settings', listener),
    setWindowDark: (dark) => invoke('window_set_dark', { dark }),

    sessionsList: (mode) => invoke('sessions_list', { mode }),
    sessionCreate: (mode) => invoke('session_create', { mode }),
    sessionGet: (id) => invoke('session_get', { id }),
    sessionUpdate: (id, patch) => invoke('session_update', { id, patch }),
    sessionDelete: (id) => invoke('session_delete', { id }),
    branchSelect: (sessionId, messageId) => invoke('branch_select', { sessionId, messageId }),
    search: (query, mode) => invoke('search', { query, mode: mode ?? null }),
    sessionExport: (id, format) => invoke('session_export', { id, format }),
    connectionsList: () => invoke('connections_list'),
    connectionSave: (input) => invoke('connection_save', { input }),
    connectionDelete: (id) => invoke('connection_delete', { id }),
    connectionModels: (id) => invoke('connection_models', { id }),
    connectionTest: (baseUrl, apiKey) =>
      invoke('connection_test', { baseUrl, apiKey: apiKey ?? null }),
    runStart: (sessionId, action, onEvent) =>
      invoke('run_start', { args: { sessionId, action }, channel: channel(onEvent) }),
    runAttach: (runId, onEvent) => invoke('run_attach', { runId, channel: channel(onEvent) }),
    runCancel: (runId) => invoke('run_cancel', { runId }),
    runsActive: () => invoke('runs_active'),
    onSessionsChanged: (listener) => subscribe<null>('hearth://sessions', () => listener()),
    onRunState: (listener) => subscribe<RunInfo>('hearth://run-state', listener),
    openLink: (url) => invoke('open_link', { url }),

    window: {
      minimize: () => void win.minimize(),
      toggleMaximize: () => void win.toggleMaximize(),
      close: () => void win.close(),
      isMaximized: () => win.isMaximized(),
      onResized: (listener) => unsubscriber(win.onResized(() => listener())),
      reportMaximizeRect: (rect) => void invoke('caption_maximize_rect', { rect }),
      onCaption: (listener) => subscribe<CaptionState>('hearth://caption', listener),
    },
  };
}

/** In-memory backend for unit tests and for previewing the UI in a normal browser. */
export function fakeBackend(initial: Partial<Settings> = {}): Backend {
  let settings: Settings = { ...DEFAULT_SETTINGS, ...initial };
  const listeners = new Set<(settings: Settings) => void>();
  const noop = () => {};
  return {
    kind: 'fake',
    boot: () => ({ settings, systemDark: systemDark() }),
    appReady: async () => {},
    systemInfo: async () => ({
      appVersion: '0.1.0',
      coreVersion: '0.1.0',
      webviewVersion: 'browser preview',
      dataDir: '(in memory)',
    }),
    settingsGet: async () => settings,
    settingsSet: async (patch) => {
      const next: Record<string, unknown> = { ...settings };
      for (const [key, value] of Object.entries(patch)) {
        const fallback = DEFAULT_SETTINGS[key as keyof Settings];
        if (value === null) next[key] = fallback;
        else if (typeof value === 'object' && !Array.isArray(value) && typeof fallback === 'object')
          next[key] = { ...(next[key] as object), ...value };
        else next[key] = value;
      }
      settings = next as Settings;
      listeners.forEach((listener) => listener(settings));
      return settings;
    },
    onSettings: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    setWindowDark: async () => {},
    ...fakeChat(),
    window: {
      minimize: noop,
      toggleMaximize: noop,
      close: noop,
      isMaximized: async () => false,
      onResized: () => noop,
      reportMaximizeRect: noop,
      onCaption: () => noop,
    },
  };
}

let current: Backend | undefined;

/** The backend for this page: Tauri inside the app, the fake anywhere else. */
export function backend(): Backend {
  current ??=
    typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
      ? tauriBackend()
      : fakeBackend();
  return current;
}

/** Tests swap in their own backend. */
export function setBackend(next: Backend) {
  current = next;
}

/** A readable message from whatever a backend call threw. */
export function errorMessage(err: unknown): string {
  if (err && typeof err === 'object' && 'message' in err) return String(err.message);
  return String(err);
}
