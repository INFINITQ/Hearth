// How a run's events change what the transcript shows. Pure, so it is tested on its own.

import type { Message, RunEnvelope } from './types';

export type LiveRun = {
  runId: string | null;
  sessionId: string;
  /** The reply being written. */
  messageId: string | null;
  status: 'starting' | 'streaming';
  /** llama.cpp reading the prompt (shown before the first token). */
  progress: { processed: number; total: number } | null;
  /** The text so far, kept so a reload of the transcript can't lose it. */
  content: string;
  reasoning: string;
};

export type RunError = { message: string; retryable: boolean };

export type TranscriptState = {
  messages: Message[];
  run: LiveRun | null;
  error: RunError | null;
};

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

/** Puts a new reply right after its parent, replacing whatever followed (a regenerated version). */
function placeReply(messages: Message[], reply: Message): Message[] {
  if (messages.some((m) => m.id === reply.id)) return messages;
  const parent = reply.parentId ? messages.findIndex((m) => m.id === reply.parentId) : -1;
  const kept = parent >= 0 ? messages.slice(0, parent + 1) : messages;
  return [...kept, reply];
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
        messages = placeReply(messages, placeholder(live.sessionId, event.messageId, null));
        messages = updateMessage(messages, event.messageId, (m) => ({
          ...m,
          content: event.content,
          reasoning: event.reasoning || null,
          status: 'streaming',
        }));
      }
      return {
        ...state,
        messages,
        run: {
          ...live,
          messageId: event.messageId,
          status: event.state === 'starting' ? 'starting' : 'streaming',
          content: event.content,
          reasoning: event.reasoning,
        },
      };
    }
    case 'prompt_progress':
      return {
        ...state,
        run: { ...live, progress: { processed: event.processed, total: event.total } },
      };
    case 'message_started':
      return {
        ...state,
        messages: placeReply(
          state.messages,
          placeholder(live.sessionId, event.messageId, event.parentId),
        ),
        run: {
          ...live,
          messageId: event.messageId,
          status: 'streaming',
          content: '',
          reasoning: '',
        },
      };
    case 'reasoning_delta':
      return {
        ...state,
        messages: updateMessage(state.messages, event.messageId, (m) => ({
          ...m,
          reasoning: (m.reasoning ?? '') + event.text,
        })),
        run: { ...live, reasoning: live.reasoning + event.text },
      };
    case 'content_delta':
      return {
        ...state,
        messages: updateMessage(state.messages, event.messageId, (m) => ({
          ...m,
          content: m.content + event.text,
        })),
        run: { ...live, content: live.content + event.text, progress: null },
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
        })),
        run: live,
      };
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
