import { describe, expect, it } from 'vitest';
import { applyRunEvent, newRun, type TranscriptState, withLiveText, withNewer } from './runReducer';
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

describe('summarizing a long conversation', () => {
  it('shows the summary after the newest message and keeps the context count', () => {
    let state = play(
      start([
        message('u1', 'user', 'Plan a trip', null),
        message('a1', 'assistant', 'A long plan', 'u1'),
        message('u2', 'user', 'Which day?', 'a1'),
      ]),
      [
        event({ type: 'started', sessionId: 's', userMessageId: 'u2', model: 'm' }),
        event({ type: 'compacting' }),
      ],
    );
    expect(state.run?.compacting).toBe(true);
    const summary = { ...message('sum1', 'summary', '- A trip plan', 'u2'), coversUntil: 'a1' };
    state = play(state, [
      event({ type: 'compacted', message: summary }),
      event({ type: 'message_started', messageId: 'a2', parentId: 'sum1' }),
      event({ type: 'context', used: 480, limit: 2048 }),
    ]);
    expect(state.run?.compacting).toBe(false);
    // The summary stands in for earlier messages in requests; the transcript keeps them all.
    expect(state.messages.map((m) => m.id)).toEqual(['u1', 'a1', 'u2', 'sum1', 'a2']);
    expect(state.run?.context).toEqual({ used: 480, limit: 2048 });
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

describe('withNewer', () => {
  const user = message('u1', 'user', 'Fix add()', null);
  const read = message('a1', 'assistant', '', 'u1');
  const answer = message('t1', 'tool', 'def add…', 'a1');
  const edit = message('a2', 'assistant', 'Fixing that.', 't1');
  const call = {
    id: 'c2',
    type: 'function' as const,
    function: { name: 'edit_file', arguments: '{}' },
  };

  it('keeps what a run showed while the transcript was being read', () => {
    // Read while the edit step was still being written; it finished with its call meanwhile.
    const saved = [user, read, answer, { ...edit, status: 'streaming' as const }];
    const shown = [{ ...user, id: 'pending-1' }, read, answer, { ...edit, toolCalls: [call] }];
    const merged = withNewer(saved, shown);
    expect(merged.map((m) => m.id)).toEqual(['u1', 'a1', 't1', 'a2']);
    expect(merged[3]!.toolCalls).toEqual([call]);
    expect(merged[3]!.status).toBe('complete');
  });

  it('keeps steps that arrived after the transcript was read', () => {
    const saved = [user, read];
    const shown = [{ ...user, id: 'pending-1' }, read, answer, edit];
    expect(withNewer(saved, shown).map((m) => m.id)).toEqual(['u1', 'a1', 't1', 'a2']);
  });

  it('a regenerated reply replaces the old one, whichever side saw it first', () => {
    const old = message('a9', 'assistant', 'Old reply', 'u1');
    const fresh = message('a10', 'assistant', 'New reply', 'u1');
    // Its event arrived before the read saw it…
    expect(withNewer([user, old], [user, fresh]).map((m) => m.id)).toEqual(['u1', 'a10']);
    // …or the read saw it before its event arrived: the old version isn't brought back.
    const known = { ...fresh, versions: { index: 1, ids: ['a9', 'a10'] } };
    expect(withNewer([user, known], [user, old]).map((m) => m.id)).toEqual(['u1', 'a10']);
  });

  it('leaves out messages that no longer continue the transcript', () => {
    const stray = message('x1', 'assistant', 'From another branch', 'gone');
    expect(withNewer([user, read], [user, read, stray]).map((m) => m.id)).toEqual(['u1', 'a1']);
  });
});
