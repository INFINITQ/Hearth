import { describe, expect, it } from 'vitest';
import { agentCommands, answersByStep, callTitle, canonicalTool, latestTodos } from './toolCalls';
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
});
