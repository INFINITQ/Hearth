// The terminal panel under coding sessions: its shells (they run in Rust), which tab shows, and
// how tall it is.

import { create } from 'zustand';
import { backend, errorMessage } from '../lib/backend';
import type { ShellKind, TerminalInfo } from '../lib/types';
import { useChat } from './chat';

/** The tab with the output of the agent's commands. */
export const AGENT_TAB = 'agent';
export const MIN_HEIGHT = 140;

type TerminalState = {
  open: boolean;
  height: number;
  /** Every running terminal; the panel shows the current project's. */
  terminals: TerminalInfo[];
  /** The tab shown: a terminal's id, or the Agent tab. */
  active: string;
  /** Whether the shell just shown takes the keyboard focus (not while arrowing through tabs). */
  focusShown: boolean;
  error: string | null;
  /** Opening the panel starts a shell when the project has none. */
  setOpen(open: boolean): void;
  toggle(): void;
  setHeight(height: number): void;
  /** Terminals that kept running while the page reloaded. */
  load(): Promise<void>;
  create(shell?: ShellKind | null): Promise<void>;
  /** `focus`: the shell shown next takes the focus (a click), or not (a key on the tabs). */
  close(id: string, focus?: boolean): Promise<void>;
  select(id: string, focus?: boolean): void;
  exited(id: string, code: number | null): void;
  dismissError(): void;
};

const projectId = () => useChat.getState().project?.id ?? null;

export const useTerminal = create<TerminalState>()((set, get) => ({
  open: false,
  height: 280,
  terminals: [],
  active: AGENT_TAB,
  focusShown: true,
  error: null,

  setOpen: (open) => {
    set({ open });
    const project = projectId();
    if (open && project && !get().terminals.some((t) => t.projectId === project)) {
      void get().create();
    }
  },
  toggle: () => get().setOpen(!get().open),
  setHeight: (height) => set({ height: Math.max(MIN_HEIGHT, Math.round(height)) }),

  load: async () => {
    try {
      const terminals = await backend().terminalsList();
      set({ terminals });
    } catch {
      // No terminals to show is fine.
    }
  },

  create: async (shell = null) => {
    try {
      // The real size follows as soon as the terminal is on screen.
      const info = await backend().terminalOpen(shell, projectId(), 100, 16);
      set((s) => ({
        terminals: [...s.terminals, info],
        active: info.id,
        focusShown: true,
        error: null,
        open: true,
      }));
    } catch (err) {
      set({ error: errorMessage(err), active: AGENT_TAB });
    }
  },

  close: async (id, focus = true) => {
    const { terminals, active } = get();
    const closing = terminals.find((t) => t.id === id);
    const rest = terminals.filter((t) => t.id !== id);
    const sameProject = rest.filter((t) => t.projectId === closing?.projectId);
    set({
      terminals: rest,
      active: active === id ? (sameProject.at(-1)?.id ?? AGENT_TAB) : active,
      focusShown: focus,
    });
    await backend()
      .terminalClose(id)
      .catch(() => {});
  },

  select: (id, focus = true) => set({ active: id, focusShown: focus }),
  exited: (id, code) =>
    set((s) => ({
      terminals: s.terminals.map((t) =>
        t.id === id ? { ...t, running: false, exitCode: code } : t,
      ),
    })),
  dismissError: () => set({ error: null }),
}));
