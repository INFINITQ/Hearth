import { describe, expect, it } from 'vitest';
import { applyRunEvent, type TranscriptState, withLiveText } from './runReducer';
import type { Message, RunEnvelope, RunEvent } from './types';

function message(
  id: string,
  role: Message['role'],
  content: string,
  parentId: string | null,
): Message {
  return {
    id,
    sessionId: 's',
    parentId,
    role,
    content,
    reasoning: null,
    reasoningMs: null,
    modelLabel: null,
    finishReason: null,
    status: 'complete',
    error: null,
    usage: null,
    createdAt: 0,
    versions: { index: 0, ids: [id] },
  };
}

let seq = 0;
function event(e: RunEvent, runId = 'r1'): RunEnvelope {
  return { ...e, runId, seq: ++seq } as RunEnvelope;
}

function start(messages: Message[]): TranscriptState {
  return {
    messages,
    run: {
      runId: null,
      sessionId: 's',
      messageId: null,
      status: 'starting',
      progress: null,
      content: '',
      reasoning: '',
    },
    error: null,
  };
}

function play(state: TranscriptState, events: RunEnvelope[]) {
  return events.reduce(applyRunEvent, state);
}

describe('applyRunEvent', () => {
  it('streams a reply after its user message and finishes the run', () => {
    const state = play(start([message('u1', 'user', 'Hi', null)]), [
      event({ type: 'started', sessionId: 's', userMessageId: 'u1', model: 'm' }),
      event({ type: 'message_started', messageId: 'a1', parentId: 'u1' }),
      event({ type: 'reasoning_delta', messageId: 'a1', text: 'Think' }),
      event({ type: 'content_delta', messageId: 'a1', text: 'Hel' }),
      event({ type: 'content_delta', messageId: 'a1', text: 'lo' }),
    ]);
    expect(state.messages.map((m) => [m.id, m.content])).toEqual([
      ['u1', 'Hi'],
      ['a1', 'Hello'],
    ]);
    expect(state.messages[1]!.reasoning).toBe('Think');
    expect(state.run?.runId).toBe('r1');

    const done = play(state, [
      event({
        type: 'message_finished',
        messageId: 'a1',
        status: 'complete',
        finishReason: 'stop',
        usage: null,
        reasoningMs: 900,
      }),
      event({ type: 'finished', status: 'completed' }),
    ]);
    expect(done.run).toBeNull();
    expect(done.messages[1]!.status).toBe('complete');
    expect(done.messages[1]!.reasoningMs).toBe(900);
  });

  it('a regenerated reply replaces the old version in place', () => {
    const state = play(
      start([message('u1', 'user', 'Hi', null), message('a1', 'assistant', 'Old', 'u1')]),
      [
        event({ type: 'message_started', messageId: 'a2', parentId: 'u1' }),
        event({ type: 'content_delta', messageId: 'a2', text: 'New' }),
      ],
    );
    expect(state.messages.map((m) => m.id)).toEqual(['u1', 'a2']);
  });

  it('ignores events from another run and records errors', () => {
    let state = play(start([]), [
      event({ type: 'started', sessionId: 's', userMessageId: null, model: 'm' }),
    ]);
    state = applyRunEvent(
      state,
      event({ type: 'content_delta', messageId: 'x', text: 'nope' }, 'other'),
    );
    expect(state.messages).toEqual([]);
    state = applyRunEvent(
      state,
      event({ type: 'error', code: 'http', message: 'boom', retryable: true }),
    );
    expect(state.error).toEqual({ message: 'boom', retryable: true });
  });

  it('a snapshot after re-attaching restores the reply so far', () => {
    const state = play(start([message('u1', 'user', 'Hi', null)]), [
      event({
        type: 'snapshot',
        sessionId: 's',
        state: 'streaming',
        userMessageId: 'u1',
        messageId: 'a1',
        content: 'Partial',
        reasoning: '',
      }),
    ]);
    expect(state.messages.at(-1)).toMatchObject({
      id: 'a1',
      content: 'Partial',
      status: 'streaming',
    });
  });
});

describe('withLiveText', () => {
  it('keeps the newest streamed text over an older saved copy', () => {
    const saved = [message('a1', 'assistant', 'Hel', null)];
    const run = {
      runId: 'r',
      sessionId: 's',
      messageId: 'a1',
      status: 'streaming' as const,
      progress: null,
      content: 'Hello',
      reasoning: '',
    };
    expect(withLiveText(saved, run)[0]!.content).toBe('Hello');
    expect(withLiveText(saved, null)[0]!.content).toBe('Hel');
  });
});
