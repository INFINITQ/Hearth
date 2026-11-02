import { create } from 'zustand';
import { backend, errorMessage } from '../lib/backend';
import { type LiveRun, type RunError, applyRunEvent, withLiveText } from '../lib/runReducer';
import type {
  ChatAction,
  ChatParams,
  Message,
  Mode,
  ModelRef,
  RunEnvelope,
  SearchHit,
  Session,
  SessionSummary,
} from '../lib/types';

type Draft = { params: ChatParams; persona: string | null };

type ChatState = {
  lists: Record<Mode, SessionSummary[]>;
  /** Sessions with a reply being written, for sidebar spinners: sessionId → runId. */
  busy: Record<string, string>;
  /** The open conversation (null: a new chat that hasn't started). */
  session: Session | null;
  messages: Message[];
  run: LiveRun | null;
  error: RunError | null;
  /** Settings for a chat that hasn't started yet. */
  draft: Draft;
  query: string;
  hits: SearchHit[];

  loadLists(): Promise<void>;
  open(id: string): Promise<void>;
  newChat(): void;
  send(text: string, mode: Mode): Promise<void>;
  stop(): void;
  regenerate(messageId: string): Promise<void>;
  edit(messageId: string, text: string): Promise<void>;
  showVersion(message: Message, step: -1 | 1): Promise<void>;
  rename(id: string, title: string): Promise<void>;
  setPinned(id: string, pinned: boolean): Promise<void>;
  remove(id: string): Promise<void>;
  setModel(model: ModelRef): Promise<void>;
  setParams(params: ChatParams, persona?: string | null): Promise<void>;
  search(query: string, mode: Mode): Promise<void>;
  setBusy(sessionId: string, runId: string | null): void;
  dismissError(): void;
};

const NO_DRAFT: Draft = { params: {}, persona: null };

function handle(event: RunEnvelope) {
  const state = useChat.getState();
  if (event.type === 'title_changed') {
    useChat.setState({
      session:
        state.session?.id === event.sessionId
          ? { ...state.session, title: event.title }
          : state.session,
    });
    void state.loadLists();
  }
  const next = applyRunEvent(
    { messages: state.messages, run: state.run, error: state.error },
    event,
  );
  if (next !== undefined) useChat.setState(next);
  if (event.type === 'finished' && state.run && state.session) {
    void refresh(state.session.id);
    void state.loadLists();
  }
}

/** Reloads the open conversation from the database, keeping any live reply's newest text. */
async function refresh(id: string) {
  const view = await backend().sessionGet(id);
  const { session, run } = useChat.getState();
  if (session?.id !== id) return;
  useChat.setState({ session: view.session, messages: withLiveText(view.messages, run) });
}

function pendingUser(sessionId: string, text: string, parentId: string | null): Message {
  const id = `pending-${Date.now()}`;
  return {
    id,
    sessionId,
    parentId,
    role: 'user',
    content: text,
    reasoning: null,
    reasoningMs: null,
    modelLabel: null,
    finishReason: null,
    status: 'complete',
    error: null,
    usage: null,
    createdAt: Date.now(),
    versions: { index: 0, ids: [id] },
  };
}

async function startRun(sessionId: string, action: ChatAction) {
  useChat.setState({
    run: {
      runId: null,
      sessionId,
      messageId: null,
      status: 'starting',
      progress: null,
      content: '',
      reasoning: '',
    },
    error: null,
  });
  try {
    await backend().runStart(sessionId, action, handle);
    await refresh(sessionId);
  } catch (err) {
    useChat.setState({ run: null, error: { message: errorMessage(err), retryable: false } });
    await refresh(sessionId).catch(() => {});
  }
}

export const useChat = create<ChatState>()((set, get) => ({
  lists: { chat: [], code: [] },
  busy: {},
  session: null,
  messages: [],
  run: null,
  error: null,
  draft: NO_DRAFT,
  query: '',
  hits: [],

  loadLists: async () => {
    const [chat, code] = await Promise.all([
      backend().sessionsList('chat'),
      backend().sessionsList('code'),
    ]);
    set({ lists: { chat, code } });
  },

  open: async (id) => {
    const view = await backend().sessionGet(id);
    set({ session: view.session, messages: view.messages, run: null, error: null });
    if (view.run) {
      set({
        run: {
          runId: view.run.runId,
          sessionId: id,
          messageId: null,
          status: 'starting',
          progress: null,
          content: '',
          reasoning: '',
        },
      });
      await backend().runAttach(view.run.runId, handle);
    }
  },

  newChat: () => set({ session: null, messages: [], run: null, error: null, draft: NO_DRAFT }),

  send: async (text, mode) => {
    let session = get().session;
    if (!session || session.mode !== mode) {
      session = await backend().sessionCreate(mode);
      const { params, persona } = get().draft;
      if (Object.keys(params).length || persona) {
        session = await backend().sessionUpdate(session.id, { params, persona });
      }
      set({ session, messages: [], draft: NO_DRAFT });
    }
    const leaf = get().messages.at(-1)?.id ?? null;
    set({ messages: [...get().messages, pendingUser(session.id, text.trim(), leaf)] });
    await startRun(session.id, { kind: 'send', text });
  },

  stop: () => {
    const run = get().run;
    if (run?.runId) void backend().runCancel(run.runId);
  },

  regenerate: async (messageId) => {
    const session = get().session;
    if (session) await startRun(session.id, { kind: 'regenerate', messageId });
  },

  edit: async (messageId, text) => {
    const session = get().session;
    if (!session || !text.trim()) return;
    const index = get().messages.findIndex((m) => m.id === messageId);
    const original = get().messages[index];
    if (original) {
      set({
        messages: [
          ...get().messages.slice(0, index),
          pendingUser(session.id, text.trim(), original.parentId),
        ],
      });
    }
    await startRun(session.id, { kind: 'edit', messageId, text });
  },

  showVersion: async (message, step) => {
    const session = get().session;
    const target = message.versions.ids[message.versions.index + step];
    if (!session || !target || get().run) return;
    const view = await backend().branchSelect(session.id, target);
    set({ session: view.session, messages: view.messages });
  },

  rename: async (id, title) => {
    const session = await backend().sessionUpdate(id, { title });
    if (get().session?.id === id) set({ session });
    await get().loadLists();
  },

  setPinned: async (id, pinned) => {
    await backend().sessionUpdate(id, { pinned });
    await get().loadLists();
  },

  remove: async (id) => {
    await backend().sessionDelete(id);
    if (get().session?.id === id) get().newChat();
    await get().loadLists();
  },

  setModel: async (model) => {
    const current = get().session;
    if (!current) return;
    const session = await backend().sessionUpdate(current.id, { modelRef: model });
    set({ session });
  },

  setParams: async (params, persona) => {
    const current = get().session;
    if (!current) {
      set({ draft: { params, persona: persona === undefined ? get().draft.persona : persona } });
      return;
    }
    const patch = persona === undefined ? { params } : { params, persona };
    const session = await backend().sessionUpdate(current.id, patch);
    set({ session });
  },

  search: async (query, mode) => {
    set({ query });
    if (!query.trim()) {
      set({ hits: [] });
      return;
    }
    const hits = await backend().search(query, mode);
    if (get().query === query) set({ hits });
  },

  setBusy: (sessionId, runId) => {
    const busy = { ...get().busy };
    if (runId) busy[sessionId] = runId;
    else delete busy[sessionId];
    set({ busy });
  },

  dismissError: () => set({ error: null }),
}));
