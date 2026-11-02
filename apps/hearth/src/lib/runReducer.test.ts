import { describe, expect, it } from 'vitest';
import { applyRunEvent, newRun, type TranscriptState, withLiveText } from './runReducer';
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
  return { messages, run: newRun('s'), error: null, notice: null };
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

describe('applyRunEvent in coding sessions', () => {
  const call = {
    id: 'c1',
    type: 'function' as const,
    function: { name: 'edit_file', arguments: '{}' },
  };

  it('a step ends with its tool calls and cleaned-up text, then the answer follows it', () => {
    let state = play(start([message('u1', 'user', 'Fix it', null)]), [
      event({ type: 'message_started', messageId: 'a1', parentId: 'u1' }),
      event({ type: 'content_delta', messageId: 'a1', text: 'Editing <tool_call>…' }),
      event({
        type: 'tool_call_progress',
        messageId: 'a1',
        index: 0,
        name: 'edit_file',
        chars: 120,
      }),
    ]);
    expect(state.run?.writing).toEqual({ name: 'edit_file', chars: 120 });
    state = play(state, [
      event({
        type: 'message_finished',
        messageId: 'a1',
        status: 'complete',
        finishReason: 'tool_calls',
        usage: null,
        reasoningMs: null,
        content: 'Editing',
        toolCalls: [call],
      }),
      event({ type: 'tool_started', callId: 'c1', tool: 'edit_file', title: 'Edit a.py' }),
    ]);
    expect(state.messages[1]).toMatchObject({ content: 'Editing', toolCalls: [call] });
    expect(state.run).toMatchObject({ messageId: null, writing: null, tool: { callId: 'c1' } });

    const answer = {
      ...message('t1', 'tool', 'Edited a.py', 'a1'),
      toolCallId: 'c1',
      toolStatus: 'ok' as const,
    };
    state = applyRunEvent(
      state,
      event({
        type: 'tool_finished',
        callId: 'c1',
        status: 'ok',
        summary: 'Edited a.py',
        message: answer,
      }),
    );
    expect(state.messages.map((m) => m.id)).toEqual(['u1', 'a1', 't1']);
    expect(state.run?.tool).toBeNull();
  });

  it('a permission prompt waits, and answering it clears the prompt', () => {
    let state = play(start([]), [
      event({
        type: 'permission_request',
        requestId: 'p1',
        callId: 'c1',
        tool: 'edit_file',
        class: 'edit',
        title: 'Edit a.py',
        path: 'a.py',
        diff: '-a\n+b',
        command: null,
        danger: null,
        choices: [{ id: 'allow_once', label: 'Allow' }],
      }),
    ]);
    expect(state.run).toMatchObject({
      status: 'waiting',
      permission: { requestId: 'p1', diff: '-a\n+b' },
    });
    state = applyRunEvent(
      state,
      event({ type: 'permission_resolved', requestId: 'p1', decision: 'allow_once' }),
    );
    expect(state.run).toMatchObject({ status: 'streaming', permission: null });

    state = play(state, [
      event({
        type: 'question',
        questionId: 'q1',
        callId: 'c2',
        question: 'Tabs?',
        options: ['Yes', 'No'],
      }),
      event({ type: 'notice', level: 'info', text: 'Paused after 50 steps.' }),
    ]);
    expect(state.run?.question?.options).toEqual(['Yes', 'No']);
    expect(state.notice?.text).toBe('Paused after 50 steps.');
  });
});

describe('commands and messages sent mid-task', () => {
  it('collects live output per call and places steered messages', () => {
    let state = start([
      message('u1', 'user', 'Run tests', null),
      message('a1', 'assistant', '', 'u1'),
    ]);
    state = play(state, [
      event({ type: 'tool_output', callId: 'c1', text: 'running 3 tests' }),
      event({ type: 'tool_output', callId: 'c1', text: 'ok' }),
    ]);
    expect(state.run?.output.c1).toBe('running 3 tests\nok\n');

    state = { ...state, run: { ...state.run!, queued: ['Also lint', 'And format'] } };
    const steer = { ...message('u2', 'user', 'Also lint', 'a1'), turn: 1 };
    state = applyRunEvent(state, event({ type: 'steered', message: steer }));
    expect(state.messages.map((m) => m.id)).toEqual(['u1', 'a1', 'u2']);
    expect(state.run?.queued).toEqual(['And format']);
  });
});

describe('withLiveText', () => {
  it('keeps the newest streamed text over an older saved copy', () => {
    const saved = [message('a1', 'assistant', 'Hel', null)];
    const run = {
      ...newRun('s', 'r'),
      messageId: 'a1',
      status: 'streaming' as const,
      content: 'Hello',
    };
    expect(withLiveText(saved, run)[0]!.content).toBe('Hello');
    expect(withLiveText(saved, null)[0]!.content).toBe('Hel');
  });
});
