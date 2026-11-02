import { create } from 'zustand';
import { backend, errorMessage } from '../lib/backend';
import {
  type LiveRun,
  type RunError,
  type RunNotice,
  applyRunEvent,
  newRun,
  withLiveText,
} from '../lib/runReducer';
import type {
  ChatAction,
  ChatParams,
  Message,
  Mode,
  ModelRef,
  PermissionChoice,
  PermissionMode,
  Project,
  RunEnvelope,
  RunInfo,
  SearchHit,
  Session,
  SessionSummary,
} from '../lib/types';

type Draft = { params: ChatParams; persona: string | null };

type ChatState = {
  lists: Record<Mode, SessionSummary[]>;
  /** Sessions with a run in progress, for sidebar spinners: sessionId → runId. */
  busy: Record<string, string>;
  /** Sessions whose run is waiting for the user (a permission prompt or a question). */
  waiting: Record<string, boolean>;
  /** The open conversation (null: a new one that hasn't started). */
  session: Session | null;
  messages: Message[];
  run: LiveRun | null;
  error: RunError | null;
  notice: RunNotice | null;
  /** Settings for a chat that hasn't started yet. */
  draft: Draft;
  /** The permission mode for a coding session that hasn't started yet. */
  draftMode: PermissionMode;
  query: string;
  hits: SearchHit[];
  /** Code mode: the open project, the recent ones, and one waiting to be trusted. */
  project: Project | null;
  projects: Project[];
  pendingTrust: Project | null;

  loadLists(): Promise<void>;
  loadProjects(): Promise<void>;
  pickProject(): Promise<void>;
  openProject(project: Project): Promise<void>;
  trustPending(): Promise<void>;
  cancelTrust(): void;
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
  setPermissionMode(mode: PermissionMode): Promise<void>;
  answerPermission(decision: PermissionChoice['id'], note?: string): Promise<void>;
  answerQuestion(answer: string): Promise<void>;
  search(query: string, mode: Mode): Promise<void>;
  setRunState(info: RunInfo): void;
  dismissError(): void;
  dismissNotice(): void;
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
  useChat.setState(
    applyRunEvent(
      { messages: state.messages, run: state.run, error: state.error, notice: state.notice },
      event,
    ),
  );
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
  useChat.setState({ run: newRun(sessionId), error: null, notice: null });
  try {
    await backend().runStart(sessionId, action, handle);
    await refresh(sessionId);
  } catch (err) {
    useChat.setState({ run: null, error: { message: errorMessage(err), retryable: false } });
    await refresh(sessionId).catch(() => {});
  }
}

/** Switches the Code side to a project (its sessions, a fresh session). */
async function selectProject(project: Project) {
  const { session } = useChat.getState();
  const keep = session?.mode === 'code' && session.workspaceId === project.id;
  useChat.setState({
    project,
    ...(keep || session?.mode === 'chat'
      ? {}
      : { session: null, messages: [], run: null, error: null, notice: null }),
  });
  await Promise.all([useChat.getState().loadLists(), useChat.getState().loadProjects()]);
}

export const useChat = create<ChatState>()((set, get) => ({
  lists: { chat: [], code: [] },
  busy: {},
  waiting: {},
  session: null,
  messages: [],
  run: null,
  error: null,
  notice: null,
  draft: NO_DRAFT,
  draftMode: 'auto_edit',
  query: '',
  hits: [],
  project: null,
  projects: [],
  pendingTrust: null,

  loadLists: async () => {
    const project = get().project;
    const [chat, code] = await Promise.all([
      backend().sessionsList('chat'),
      project ? backend().projectSessions(project.id) : Promise.resolve([]),
    ]);
    set({ lists: { chat, code } });
  },

  loadProjects: async () => {
    const projects = await backend().projectsRecent();
    set({ projects });
    if (!get().project) {
      // Reopen the last project, if it's still there and was trusted.
      const last = projects.find((p) => p.exists && p.trusted);
      if (last) await selectProject(last);
    }
  },

  pickProject: async () => {
    try {
      const picked = await backend().projectPick();
      if (!picked) return;
      if (picked.trusted) await selectProject(picked);
      else set({ pendingTrust: picked });
    } catch (err) {
      set({ error: { message: errorMessage(err), retryable: false } });
    }
  },

  openProject: async (project) => {
    try {
      const opened = await backend().projectOpen(project.path);
      if (opened.trusted) await selectProject(opened);
      else set({ pendingTrust: opened });
    } catch (err) {
      set({ error: { message: errorMessage(err), retryable: false } });
      await get().loadProjects();
    }
  },

  trustPending: async () => {
    const pending = get().pendingTrust;
    if (!pending) return;
    const trusted = await backend().projectTrust(pending.id);
    set({ pendingTrust: null });
    await selectProject(trusted);
  },

  cancelTrust: () => set({ pendingTrust: null }),

  open: async (id) => {
    const view = await backend().sessionGet(id);
    set({ session: view.session, messages: view.messages, run: null, error: null, notice: null });
    const project = get().project;
    if (view.session.mode === 'code' && view.session.workspaceId !== project?.id) {
      const owner = get().projects.find((p) => p.id === view.session.workspaceId);
      if (owner) {
        set({ project: owner });
        void get().loadLists();
      }
    }
    if (view.run) {
      set({ run: newRun(id, view.run.runId) });
      await backend().runAttach(view.run.runId, handle);
    }
  },

  newChat: () =>
    set({ session: null, messages: [], run: null, error: null, notice: null, draft: NO_DRAFT }),

  send: async (text, mode) => {
    let session = get().session;
    if (!session || session.mode !== mode) {
      if (mode === 'code') {
        const project = get().project;
        if (!project) return;
        session = await backend().codeSessionCreate(project.id, get().draftMode);
      } else {
        session = await backend().sessionCreate(mode);
        const { params, persona } = get().draft;
        if (Object.keys(params).length || persona) {
          session = await backend().sessionUpdate(session.id, { params, persona });
        }
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

  setPermissionMode: async (mode) => {
    set({ draftMode: mode });
    const current = get().session;
    if (current?.mode !== 'code') return;
    set({ session: { ...current, permissionMode: mode } });
    const session = await backend().sessionUpdate(current.id, { permissionMode: mode });
    if (get().session?.id === session.id) set({ session });
  },

  answerPermission: async (decision, note) => {
    const request = get().run?.permission;
    if (request) await backend().runPermission(request.requestId, decision, note);
  },

  answerQuestion: async (answer) => {
    const question = get().run?.question;
    if (question) await backend().runAnswer(question.questionId, answer);
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

  setRunState: (info) => {
    const busy = { ...get().busy };
    const waiting = { ...get().waiting };
    if (info.state === 'finished') {
      delete busy[info.sessionId];
      delete waiting[info.sessionId];
    } else {
      busy[info.sessionId] = info.runId;
      if (info.state === 'waiting') waiting[info.sessionId] = true;
      else delete waiting[info.sessionId];
    }
    set({ busy, waiting });
  },

  dismissError: () => set({ error: null }),
  dismissNotice: () => set({ notice: null }),
}));
