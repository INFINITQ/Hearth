// How a run's events change what the transcript shows. Pure, so it is tested on its own.

import type { Message, PermissionRequest, Question, RunEnvelope } from './types';

export type LiveRun = {
  runId: string | null;
  sessionId: string;
  /** The reply being written. */
  messageId: string | null;
  status: 'starting' | 'streaming' | 'waiting';
  /** llama.cpp reading the prompt (shown before the first token). */
  progress: { processed: number; total: number } | null;
  /** The text so far, kept so a reload of the transcript can't lose it. */
  content: string;
  reasoning: string;
  /** Coding sessions: the tool running now. */
  tool: { callId: string; title: string } | null;
  /** A tool call's arguments arriving (a long file being written). */
  writing: { name: string | null; chars: number } | null;
  /** Waiting for the user. */
  permission: PermissionRequest | null;
  question: Question | null;
  /** Output of running commands so far, by call id (the newest part). */
  output: Record<string, string>;
  /** Messages the user sent while it worked, waiting to join the conversation. */
  queued: string[];
  /** The model being loaded before the reply can start (Hearth's own llama-server). */
  loading: string | null;
  /** The earlier conversation is being summarized to make room. */
  compacting: boolean;
  /** How full the context window was after the newest reply. */
  context: { used: number; limit: number } | null;
};

/** Live command output kept per call. */
const LIVE_OUTPUT_CHARS = 20_000;

export type RunError = { message: string; retryable: boolean };
export type RunNotice = { level: 'info' | 'warning'; text: string };

export type TranscriptState = {
  messages: Message[];
  run: LiveRun | null;
  error: RunError | null;
  notice: RunNotice | null;
};

export function newRun(sessionId: string, runId: string | null = null): LiveRun {
  return {
    runId,
    sessionId,
    messageId: null,
    status: 'starting',
    progress: null,
    content: '',
    reasoning: '',
    tool: null,
    writing: null,
    permission: null,
    question: null,
    output: {},
    queued: [],
    loading: null,
    compacting: false,
    context: null,
  };
}

function placeholder(sessionId: string, id: string, parentId: string | null): Message {
  return {
    id,
    sessionId,
    parentId,
    role: 'assistant',
    content: '',
    reasoning: null,
    reasoningMs: null,
    modelLabel: null,
    finishReason: null,
    status: 'streaming',
    error: null,
    usage: null,
    createdAt: Date.now(),
    versions: { index: 0, ids: [id] },
  };
}

function updateMessage(
  messages: Message[],
  id: string,
  change: (m: Message) => Message,
): Message[] {
  const index = messages.findIndex((m) => m.id === id);
  if (index < 0) return messages;
  const next = messages.slice();
  next[index] = change(messages[index]!);
  return next;
}

/** Puts a new message right after its parent, replacing whatever followed (a regenerated version). */
function placeAfterParent(messages: Message[], message: Message): Message[] {
  if (messages.some((m) => m.id === message.id)) return messages;
  const parent = message.parentId ? messages.findIndex((m) => m.id === message.parentId) : -1;
  const kept = parent >= 0 ? messages.slice(0, parent + 1) : messages;
  return [...kept, message];
}

export function applyRunEvent(state: TranscriptState, event: RunEnvelope): TranscriptState {
  const run = state.run;
  if (!run) return state;
  if (run.runId && event.runId !== run.runId) return state;
  const live: LiveRun = run.runId ? run : { ...run, runId: event.runId };

  switch (event.type) {
    case 'snapshot': {
      let messages = state.messages;
      if (event.messageId) {
        messages = placeAfterParent(messages, placeholder(live.sessionId, event.messageId, null));
        messages = updateMessage(messages, event.messageId, (m) => ({
          ...m,
          content: event.content,
          reasoning: event.reasoning || null,
          status: 'streaming',
        }));
      }
      // A running tool and a waiting question follow the snapshot as their own events.
      return {
        ...state,
        messages,
        run: {
          ...live,
          messageId: event.messageId,
          status:
            event.state === 'starting'
              ? 'starting'
              : event.state === 'waiting'
                ? 'waiting'
                : 'streaming',
          content: event.content,
          reasoning: event.reasoning,
          tool: null,
          permission: null,
          question: null,
        },
      };
    }
    case 'model_loading':
      return { ...state, run: { ...live, loading: event.model } };
    case 'compacting':
      return { ...state, run: { ...live, compacting: true } };
    case 'compacted':
      return {
        ...state,
        messages: placeAfterParent(state.messages, event.message),
        run: { ...live, compacting: false },
      };
    case 'context':
      return { ...state, run: { ...live, context: { used: event.used, limit: event.limit } } };
    case 'prompt_progress':
      return {
        ...state,
        run: {
          ...live,
          loading: null,
          progress: { processed: event.processed, total: event.total },
        },
      };
    case 'message_started':
      return {
        ...state,
        messages: placeAfterParent(
          state.messages,
          placeholder(live.sessionId, event.messageId, event.parentId),
        ),
        run: {
          ...live,
          messageId: event.messageId,
          status: 'streaming',
          content: '',
          reasoning: '',
          tool: null,
          writing: null,
        },
      };
    case 'reasoning_delta':
      return {
        ...state,
        messages: updateMessage(state.messages, event.messageId, (m) => ({
          ...m,
          reasoning: (m.reasoning ?? '') + event.text,
        })),
        run: { ...live, reasoning: live.reasoning + event.text, loading: null },
      };
    case 'content_delta':
      return {
        ...state,
        messages: updateMessage(state.messages, event.messageId, (m) => ({
          ...m,
          content: m.content + event.text,
        })),
        run: { ...live, content: live.content + event.text, progress: null, loading: null },
      };
    case 'message_finished':
      return {
        ...state,
        messages: updateMessage(state.messages, event.messageId, (m) => ({
          ...m,
          status: event.status,
          usage: event.usage,
          reasoningMs: event.reasoningMs,
          finishReason: event.finishReason,
          content: event.content ?? m.content,
          toolCalls: event.toolCalls?.length ? event.toolCalls : (m.toolCalls ?? null),
        })),
        run: { ...live, messageId: null, writing: null, progress: null, loading: null },
      };
    case 'tool_call_progress':
      return { ...state, run: { ...live, writing: { name: event.name, chars: event.chars } } };
    case 'tool_started':
      return {
        ...state,
        run: { ...live, tool: { callId: event.callId, title: event.title }, writing: null },
      };
    case 'tool_output': {
      const text = `${live.output[event.callId] ?? ''}${event.text}\n`;
      return {
        ...state,
        run: {
          ...live,
          output: { ...live.output, [event.callId]: text.slice(-LIVE_OUTPUT_CHARS) },
        },
      };
    }
    case 'tool_finished':
      return {
        ...state,
        messages: placeAfterParent(state.messages, event.message),
        run: { ...live, tool: live.tool?.callId === event.callId ? null : live.tool },
      };
    case 'steered': {
      const index = live.queued.indexOf(event.message.content);
      return {
        ...state,
        messages: placeAfterParent(state.messages, event.message),
        run: {
          ...live,
          queued: index < 0 ? live.queued : live.queued.filter((_, i) => i !== index),
        },
      };
    }
    case 'permission_request': {
      const permission: PermissionRequest = {
        requestId: event.requestId,
        callId: event.callId,
        tool: event.tool,
        class: event.class,
        title: event.title,
        path: event.path,
        diff: event.diff,
        command: event.command,
        danger: event.danger,
        choices: event.choices,
      };
      return { ...state, run: { ...live, status: 'waiting', permission } };
    }
    case 'permission_resolved':
      return { ...state, run: { ...live, status: 'streaming', permission: null } };
    case 'question': {
      const question: Question = {
        questionId: event.questionId,
        callId: event.callId,
        question: event.question,
        options: event.options,
      };
      return { ...state, run: { ...live, status: 'waiting', question } };
    }
    case 'question_answered':
      return { ...state, run: { ...live, status: 'streaming', question: null } };
    case 'notice':
      return { ...state, run: live, notice: { level: event.level, text: event.text } };
    case 'error':
      return { ...state, run: live, error: { message: event.message, retryable: event.retryable } };
    case 'finished':
      return { ...state, run: null };
    default:
      return { ...state, run: live };
  }
}

/** After the transcript is reloaded from the database, keeps the live reply's newest text. */
export function withLiveText(messages: Message[], run: LiveRun | null): Message[] {
  if (!run?.messageId) return messages;
  return updateMessage(messages, run.messageId, (m) =>
    run.content.length >= m.content.length
      ? { ...m, content: run.content, reasoning: run.reasoning || m.reasoning, status: 'streaming' }
      : m,
  );
}
