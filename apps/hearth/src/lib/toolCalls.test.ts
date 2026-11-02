import { describe, expect, it } from 'vitest';
import {
  agentCommands,
  answersByStep,
  callTitle,
  canonicalTool,
  latestTodos,
  sameAnswers,
  sameTurn,
  webSources,
  webTurns,
} from './toolCalls';
import type { Message, ToolCall } from './types';

const call = (name: string, args: unknown): ToolCall => ({
  id: 'c',
  type: 'function',
  function: { name, arguments: typeof args === 'string' ? args : JSON.stringify(args) },
});

function tool(id: string, callId: string, todos?: Message['toolDisplay']): Message {
  return {
    id,
    sessionId: 's',
    parentId: null,
    role: 'tool',
    content: '',
    reasoning: null,
    reasoningMs: null,
    modelLabel: null,
    finishReason: null,
    status: 'complete',
    error: null,
    usage: null,
    createdAt: 0,
    versions: { index: 0, ids: [id] },
    toolCallId: callId,
    toolDisplay: todos ?? null,
  };
}

describe('tool calls', () => {
  it('get readable titles, aliases included, even with broken arguments', () => {
    expect(callTitle(call('read_file', { path: 'src/a.rs' }))).toBe('Read src/a.rs');
    expect(callTitle(call('str_replace', { file_path: 'b.py' }))).toBe('Edit b.py');
    expect(callTitle(call('grep', { pattern: 'TODO' }))).toBe('Search for TODO');
    expect(callTitle(call('list_dir', {}))).toBe('List .');
    expect(callTitle(call('edit_file', '{"path": '))).toBe('Edit');
    expect(callTitle(call('mcp__x__y', {}))).toBe('mcp__x__y');
    expect(canonicalTool(' Read ')).toBe('read_file');
  });

  it('answers belong to their own step even when call ids repeat, and the newest to-do list wins', () => {
    const step = (id: string): Message => ({
      ...tool(id, ''),
      role: 'assistant',
      toolCallId: null,
    });
    const first = tool('t1', 'call_0', {
      title: 'todo',
      todos: [{ content: 'A', status: 'pending' }],
    });
    const second = tool('t2', 'call_0', {
      title: 'todo',
      todos: [{ content: 'A', status: 'completed' }],
    });
    const steps = answersByStep([step('a1'), first, step('a2'), second]);
    expect(steps.get('a1')?.get('call_0')?.id).toBe('t1');
    expect(steps.get('a2')?.get('call_0')?.id).toBe('t2');
    expect(latestTodos([first, second])[0]!.status).toBe('completed');
    expect(latestTodos([])).toEqual([]);
  });

  it('lists the commands the agent ran, with their output, for the Agent tab', () => {
    const step = (id: string, calls: ToolCall[]): Message => ({
      ...tool(id, ''),
      role: 'assistant',
      toolCallId: null,
      toolCalls: calls,
    });
    const ran = { ...call('Bash', { command: 'npm test' }), id: 'c1' };
    const read = { ...call('read_file', { path: 'a.txt' }), id: 'c2' };
    const waiting = { ...call('run_command', { cmd: 'ls' }), id: 'c3' };
    const answer = {
      ...tool('t1', 'c1', {
        title: 'Run npm test',
        command: 'npm test',
        output: '3 passed',
        exitCode: 0,
        durationMs: 1200,
      }),
      toolStatus: 'ok' as const,
    };
    const commands = agentCommands([step('a1', [ran, read]), answer, step('a2', [waiting])]);
    expect(commands).toEqual([
      {
        callId: 'c1',
        command: 'npm test',
        output: '3 passed',
        exitCode: 0,
        durationMs: 1200,
        status: 'ok',
      },
      {
        callId: 'c3',
        command: 'ls',
        output: null,
        exitCode: null,
        durationMs: null,
        status: 'pending',
      },
    ]);
    expect(canonicalTool('kill_shell')).toBe('kill_process');
  });

  it('groups a chat reply that searched, with the pages it read first among its sources', () => {
    const user = { ...tool('u1', ''), role: 'user' as const, toolCallId: null };
    const step = (id: string, calls: ToolCall[]): Message => ({
      ...tool(id, ''),
      role: 'assistant',
      toolCallId: null,
      toolCalls: calls,
    });
    const searched = {
      ...tool('t1', 'c1', {
        title: 'Search',
        results: [
          { title: 'A', url: 'https://a.example/', snippet: '' },
          { title: 'B', url: 'https://b.example/', snippet: '' },
        ],
      }),
      toolName: 'web_search',
      toolStatus: 'ok' as const,
    };
    const read = {
      ...tool('t2', 'c2', { title: 'Read', url: 'https://b.example', pageTitle: 'Page B' }),
      toolName: 'web_fetch',
      toolStatus: 'ok' as const,
    };
    const failed = {
      ...tool('t3', 'c3', { title: 'Read', url: 'https://c.example/' }),
      toolName: 'web_fetch',
      toolStatus: 'error' as const,
    };
    const answer = step('a3', []);
    const turns = webTurns([
      user,
      step('a1', [{ ...call('web_search', {}), id: 'c1' }]),
      searched,
      step('a2', [
        { ...call('web_fetch', {}), id: 'c2' },
        { ...call('web_fetch', {}), id: 'c3' },
      ]),
      read,
      failed,
      answer,
    ]);
    expect([...turns.keys()]).toEqual(['a3']);
    expect(turns.get('a3')?.start.id).toBe('a1');
    expect(turns.get('a3')?.sources).toEqual([
      { url: 'https://b.example', title: 'Page B', read: true },
      { url: 'https://a.example/', title: 'A', read: false },
    ]);
    // A plain reply isn't a web turn.
    expect(webTurns([user, step('a9', [])]).size).toBe(0);
    expect(webSources([])).toEqual([]);

    // Rebuilt from the same messages, turns compare equal (so finished replies skip rendering).
    const again = webTurns([
      user,
      step('a1', [{ ...call('web_search', {}), id: 'c1' }]),
      searched,
      answer,
    ]);
    expect(
      sameTurn(
        again.get('a3'),
        webTurns([user, again.get('a3')!.start, searched, answer]).get('a3'),
      ),
    ).toBe(true);
    expect(sameTurn(turns.get('a3'), again.get('a3'))).toBe(false);
    expect(sameTurn(undefined, undefined)).toBe(true);
    expect(sameTurn(turns.get('a3'), undefined)).toBe(false);
  });

  it('compares a step’s answers by the messages they hold, not the map', () => {
    const first = tool('t1', 'c1');
    const second = tool('t2', 'c2');
    const a = new Map([
      ['c1', first],
      ['c2', second],
    ]);
    expect(sameAnswers(a, new Map(a))).toBe(true);
    expect(sameAnswers(a, new Map([['c1', first]]))).toBe(false);
    expect(sameAnswers(a, new Map([...a, ['c2', { ...second }]]))).toBe(false);
    expect(sameAnswers(undefined, undefined)).toBe(true);
    expect(sameAnswers(a, undefined)).toBe(false);
  });
});
