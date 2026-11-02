import { create } from 'zustand';
import { type SettingsPatch, backend } from '../lib/backend';
import { mergePatch } from '../lib/mergePatch';
import type { Settings } from '../lib/types';

export type Mode = 'chat' | 'code';
export type Screen = Mode | 'models' | 'settings';

type AppState = {
  screen: Screen;
  /** The sidebar's Chat/Code switch, remembered while Models or Settings is open. */
  mode: Mode;
  settings: Settings;
  /** Windows' own light/dark mode, for the "system" theme. */
  systemDark: boolean;
  go(screen: Screen): void;
  setMode(mode: Mode): void;
  setSystemDark(dark: boolean): void;
  /** Settings changed elsewhere (another part of the UI, or Rust). */
  applySettings(settings: Settings): void;
  updateSettings(patch: SettingsPatch): Promise<void>;
  toggleSidebar(): void;
};

const boot = backend().boot();

export const useApp = create<AppState>()((set, get) => ({
  screen: 'chat',
  mode: 'chat',
  settings: boot.settings,
  systemDark: boot.systemDark,

  go: (screen) =>
    set(screen === 'chat' || screen === 'code' ? { screen, mode: screen } : { screen }),
  setMode: (mode) => set({ mode, screen: mode }),
  setSystemDark: (systemDark) => set({ systemDark }),
  applySettings: (settings) => set({ settings }),

  updateSettings: async (patch) => {
    // Optimistic: the UI reacts at once; Rust validates and saves, and its answer wins.
    const before = get().settings;
    set({ settings: mergePatch(before, patch) });
    try {
      set({ settings: await backend().settingsSet(patch) });
    } catch (err) {
      set({ settings: before });
      throw err;
    }
  },

  toggleSidebar: () => {
    get()
      .updateSettings({ sidebarCollapsed: !get().settings.sidebarCollapsed })
      .catch((err: unknown) => console.error('could not save the sidebar state', err));
  },
}));
