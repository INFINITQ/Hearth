import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { type Boot, DEFAULT_SETTINGS, type Settings, type SystemInfo } from './types';

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
export type SettingsPatch = { [K in keyof Settings]?: Settings[K] | null };

/**
 * Everything the UI asks of the outside world. The app uses Tauri; unit tests and a plain
 * browser (`pnpm dev:web`) use an in-memory fake.
 */
export interface Backend {
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
        next[key] = value === null ? DEFAULT_SETTINGS[key as keyof Settings] : value;
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
