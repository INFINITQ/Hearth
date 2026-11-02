import { create } from 'zustand';
import { backend, errorMessage } from '../lib/backend';
import type { ModelPreset, ModelStatus, RuntimeOverview, Telemetry } from '../lib/types';
import { useApp } from './app';

type ModelsState = {
  overview: RuntimeOverview | null;
  telemetry: Telemetry | null;
  /** The last reason a model failed to load from this page, by model id. */
  errors: Record<string, string>;
  refreshing: boolean;
  /** `refresh`: probe llama-server and scan the folders again. */
  load(refresh?: boolean): Promise<void>;
  setStatus(status: ModelStatus[]): void;
  loadModel(id: string): Promise<void>;
  unloadModel(id: string): Promise<void>;
  readTelemetry(): Promise<void>;
  chooseServer(): Promise<void>;
  addFolder(): Promise<void>;
  removeFolder(path: string): Promise<void>;
  setPreset(id: string, preset: ModelPreset | null): Promise<void>;
  setRole(role: 'chat' | 'code', id: string): Promise<void>;
};

export const useModels = create<ModelsState>()((set, get) => ({
  overview: null,
  telemetry: null,
  errors: {},
  refreshing: false,

  load: async (refresh = false) => {
    set({ refreshing: true });
    try {
      set({ overview: await backend().runtimeOverview(refresh) });
    } finally {
      set({ refreshing: false });
    }
  },

  setStatus: (status) => {
    const overview = get().overview;
    if (overview) set({ overview: { ...overview, status } });
    else void get().load();
  },

  loadModel: async (id) => {
    const errors = { ...get().errors };
    delete errors[id];
    set({ errors });
    try {
      await backend().modelLoad(id);
    } catch (err) {
      set({ errors: { ...get().errors, [id]: errorMessage(err) } });
    }
  },

  unloadModel: async (id) => {
    await backend().modelUnload(id);
  },

  readTelemetry: async () => {
    try {
      set({ telemetry: await backend().runtimeTelemetry() });
    } catch {
      // Not important enough to show.
    }
  },

  chooseServer: async () => {
    const path = await backend().pickLlamaServer();
    if (!path) return;
    await useApp.getState().updateSettings({ runtime: { llamaServerPath: path } });
    await get().load(true);
  },

  addFolder: async () => {
    const path = await backend().pickModelFolder();
    if (!path) return;
    const folders = useApp.getState().settings.runtime.modelFolders;
    if (!folders.some((f) => f.toLowerCase() === path.toLowerCase())) {
      await useApp.getState().updateSettings({ runtime: { modelFolders: [...folders, path] } });
    }
    await get().load(true);
  },

  removeFolder: async (path) => {
    const folders = useApp.getState().settings.runtime.modelFolders.filter((f) => f !== path);
    await useApp.getState().updateSettings({ runtime: { modelFolders: folders } });
    await get().load(true);
  },

  setPreset: async (id, preset) => {
    // A merge patch: `null` removes the model's preset.
    const presets = { [id]: preset } as Record<string, ModelPreset>;
    await useApp.getState().updateSettings({ runtime: { presets } });
  },

  setRole: async (role, id) => {
    await useApp.getState().updateSettings({ models: { [role]: { kind: 'local', modelId: id } } });
  },
}));

/** "17.7 GB". */
export function formatSize(bytes: number): string {
  return bytes >= 1024 ** 3
    ? `${(bytes / 1024 ** 3).toFixed(1)} GB`
    : `${Math.max(1, Math.round(bytes / 1024 ** 2))} MB`;
}

/** "262K". */
export function formatTokens(tokens: number): string {
  return tokens >= 1024 ? `${Math.round(tokens / 1024)}K` : String(tokens);
}
