// Shapes shared with the Rust side (src-tauri commands, hearth-core). Field names are camelCase
// on both sides.

export type ThemeSetting = 'system' | 'light' | 'dark';
export type Mode = 'chat' | 'code';

/** Which model to use: a model on a saved connection (managed llama-server presets join later). */
export type ModelRef = { kind: 'connection'; connectionId: string; model: string };

export type Settings = {
  theme: ThemeSetting;
  sidebarCollapsed: boolean;
  models: { chat: ModelRef | null; code: ModelRef | null };
  chat: { systemPrompt: string | null };
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
  models: { chat: null, code: null },
  chat: { systemPrompt: null },
};

export type ChatParams = {
  temperature?: number | null;
  topP?: number | null;
  topK?: number | null;
  minP?: number | null;
  maxTokens?: number | null;
  /** Let models that can think do so; null = the model's default. */
  thinking?: boolean | null;
};

export type SessionSummary = {
  id: string;
  mode: Mode;
  title: string;
  pinned: boolean;
  updatedAt: number;
};

export type Session = SessionSummary & {
  titleSource: 'none' | 'auto' | 'user';
  workspaceId: string | null;
  modelRef: ModelRef | null;
  params: ChatParams | null;
  persona: string | null;
  permissionMode: string | null;
  searchMode: string | null;
  activeLeafId: string | null;
  archived: boolean;
  createdAt: number;
};

export type SessionPatch = {
  title?: string;
  pinned?: boolean;
  archived?: boolean;
  modelRef?: ModelRef | null;
  params?: ChatParams | null;
  persona?: string | null;
  permissionMode?: PermissionMode;
};

/** How freely the coding agent may act (see hearth-core agent/permissions.rs). */
export type PermissionMode = 'plan' | 'ask' | 'auto_edit' | 'full_auto';

/** A folder coding sessions work in. */
export type Project = {
  id: string;
  path: string;
  name: string;
  /** The user trusts its contents (its AGENTS.md is followed). */
  trusted: boolean;
  createdAt: number;
  lastOpenedAt: number;
  /** The folder is still there. */
  exists: boolean;
};

export type ToolCall = {
  id: string;
  type: 'function';
  function: { name: string; arguments: string };
};

export type Todo = { content: string; status: 'pending' | 'in_progress' | 'completed' };

/** What a tool message shows (saved with it). */
export type ToolDisplay = {
  title: string;
  summary?: string;
  args?: unknown;
  path?: string;
  diff?: string;
  todos?: Todo[];
  question?: string;
  options?: string[];
  answer?: string;
  /** The user's note with a denial. */
  note?: string;
};

export type ToolStatus = 'ok' | 'error' | 'denied' | 'cancelled';

export type PermissionChoice = { id: 'allow_once' | 'allow_session' | 'deny'; label: string };

export type PermissionRequest = {
  requestId: string;
  callId: string;
  tool: string;
  class: 'read' | 'edit' | 'execute' | 'network' | 'external' | 'internal';
  title: string;
  path: string | null;
  diff: string | null;
  choices: PermissionChoice[];
};

export type Question = { questionId: string; callId: string; question: string; options: string[] };

export type Usage = {
  promptTokens: number;
  completionTokens: number;
  cachedTokens?: number;
  promptPerSecond?: number;
  predictedPerSecond?: number;
};

export type MessageStatus = 'streaming' | 'complete' | 'interrupted' | 'error';

export type Message = {
  id: string;
  sessionId: string;
  parentId: string | null;
  role: 'user' | 'assistant' | 'tool' | 'summary';
  content: string;
  reasoning: string | null;
  reasoningMs: number | null;
  modelLabel: string | null;
  finishReason: string | null;
  status: MessageStatus;
  error: string | null;
  usage: Usage | null;
  createdAt: number;
  /** This message among its siblings (edits and regenerations). */
  versions: { index: number; ids: string[] };
  /** Coding sessions: an assistant message's tool calls, and a tool message's answer. */
  toolCalls?: ToolCall[] | null;
  toolCallId?: string | null;
  toolName?: string | null;
  toolStatus?: ToolStatus | null;
  toolDisplay?: ToolDisplay | null;
};

export type RunInfo = {
  runId: string;
  sessionId: string;
  /** `waiting`: for the user (a permission prompt or a question). */
  state: 'starting' | 'streaming' | 'waiting' | 'finished';
};

export type SessionView = {
  session: Session;
  messages: Message[];
  run: RunInfo | null;
};

export type RunEvent =
  | {
      type: 'snapshot';
      sessionId: string;
      state: RunInfo['state'];
      userMessageId: string | null;
      messageId: string | null;
      content: string;
      reasoning: string;
    }
  | { type: 'started'; sessionId: string; userMessageId: string | null; model: string }
  | { type: 'prompt_progress'; processed: number; total: number; cached: number }
  | { type: 'message_started'; messageId: string; parentId: string | null }
  | { type: 'reasoning_delta'; messageId: string; text: string }
  | { type: 'content_delta'; messageId: string; text: string }
  | {
      type: 'message_finished';
      messageId: string;
      status: MessageStatus;
      finishReason: string | null;
      usage: Usage | null;
      reasoningMs: number | null;
      /** The final text when it differs from what streamed. */
      content?: string;
      toolCalls?: ToolCall[];
    }
  | {
      type: 'tool_call_progress';
      messageId: string;
      index: number;
      name: string | null;
      chars: number;
    }
  | { type: 'tool_started'; callId: string; tool: string; title: string }
  | { type: 'tool_finished'; callId: string; status: ToolStatus; summary: string; message: Message }
  | ({ type: 'permission_request' } & PermissionRequest)
  | { type: 'permission_resolved'; requestId: string; decision: PermissionChoice['id'] }
  | ({ type: 'question' } & Question)
  | { type: 'question_answered'; questionId: string; answer: string }
  | { type: 'todos'; items: Todo[] }
  | { type: 'notice'; level: 'info' | 'warning'; text: string }
  | { type: 'title_changed'; sessionId: string; title: string }
  | { type: 'error'; code: string; message: string; retryable: boolean }
  | { type: 'finished'; status: 'completed' | 'cancelled' | 'failed' | 'paused' };

export type RunEnvelope = RunEvent & { runId: string; seq: number };

export type ChatAction =
  | { kind: 'send'; text: string }
  | { kind: 'regenerate'; messageId: string }
  | { kind: 'edit'; messageId: string; text: string };

export type StartedRun = { runId: string; userMessageId: string | null };

export type Connection = {
  id: string;
  name: string;
  baseUrl: string;
  hasApiKey: boolean;
  llamaExtensions: boolean;
  createdAt: number;
};

export type ConnectionInput = {
  id?: string | null;
  name: string;
  baseUrl: string;
  /** Omit to keep the saved key; '' removes it. */
  apiKey?: string | null;
  llamaExtensions: boolean;
};

export type SearchHit = {
  sessionId: string;
  messageId: string | null;
  mode: Mode;
  title: string;
  /** Matches are wrapped in \u0002 … \u0003. */
  snippet: string;
  updatedAt: number;
};
