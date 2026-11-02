// Shapes shared with the Rust side (src-tauri/src/commands.rs, hearth-core settings).

export type ThemeSetting = 'system' | 'light' | 'dark';

export type Settings = {
  theme: ThemeSetting;
  sidebarCollapsed: boolean;
};

/** What Rust hands the page before it loads (window.__HEARTH_BOOT__). */
export type Boot = {
  settings: Settings;
  systemDark: boolean;
};

export type SystemInfo = {
  appVersion: string;
  coreVersion: string;
  webviewVersion: string;
  dataDir: string;
};

/** Errors from Rust commands arrive as `{ kind, message }`. */
export type BackendError = {
  kind: string;
  message: string;
};

export const DEFAULT_SETTINGS: Settings = {
  theme: 'system',
  sidebarCollapsed: false,
};
