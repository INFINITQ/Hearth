// Tool calls in coding sessions: their names, titles before they run, and their answers.

import type { Message, Todo, ToolCall, ToolStatus } from './types';

/** The same aliases hearth-core accepts (agent/tools.rs `canonical_name`), for display. */
const ALIASES: Record<string, string> = {
  read: 'read_file',
  cat: 'read_file',
  view: 'read_file',
  open_file: 'read_file',
  ls: 'list_dir',
  list: 'list_dir',
  list_directory: 'list_dir',
  list_files: 'list_dir',
  find_files: 'glob',
  file_search: 'glob',
  search: 'grep',
  search_file_content: 'grep',
  rg: 'grep',
  ripgrep: 'grep',
  write: 'write_file',
  create_file: 'write_file',
  write_to_file: 'write_file',
  edit: 'edit_file',
  str_replace: 'edit_file',
  replace: 'edit_file',
  replace_in_file: 'edit_file',
  str_replace_editor: 'edit_file',
  todowrite: 'todo_write',
  update_todos: 'todo_write',
  todo: 'todo_write',
  ask: 'ask_user',
  ask_followup_question: 'ask_user',
  question: 'ask_user',
  bash: 'run_command',
  shell: 'run_command',
  run_shell_command: 'run_command',
  execute_command: 'run_command',
  terminal: 'run_command',
  powershell: 'run_command',
  exec: 'run_command',
  bash_output: 'command_output',
  read_output: 'command_output',
  get_output: 'command_output',
  kill_shell: 'kill_process',
  kill_bash: 'kill_process',
  stop_process: 'kill_process',
};

export function canonicalTool(name: string): string {
  const lower = name.trim().toLowerCase();
  return ALIASES[lower] ?? lower;
}

export function parseArgs(call: ToolCall): Record<string, unknown> {
  try {
    const value: unknown = JSON.parse(call.function.arguments);
    return value && typeof value === 'object' && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : {};
  } catch {
    return {};
  }
}

/** A title for a call that hasn't started yet (Rust's own title replaces it once it does). */
export function callTitle(call: ToolCall): string {
  const args = parseArgs(call);
  const text = (...keys: string[]) => {
    for (const key of keys) if (typeof args[key] === 'string') return args[key];
    return '';
  };
  const path = text('path', 'file_path', 'filename');
  switch (canonicalTool(call.function.name)) {
    case 'read_file':
      return `Read ${path}`.trim();
    case 'list_dir':
      return `List ${path || '.'}`;
    case 'glob':
      return `Find ${text('pattern')}`.trim();
    case 'grep':
      return `Search for ${text('pattern', 'query')}`.trim();
    case 'write_file':
      return `Write ${path}`.trim();
    case 'edit_file':
      return `Edit ${path}`.trim();
    case 'todo_write':
      return 'Update the to-do list';
    case 'ask_user':
      return 'Ask you a question';
    default:
      return call.function.name;
  }
}

/**
 * Each assistant step's answers, by the call they answer. Answers belong to the step they
 * follow, so call ids repeated across steps (some servers reuse `call_0`) can't mix them up.
 */
export function answersByStep(messages: Message[]): Map<string, Map<string, Message>> {
  const steps = new Map<string, Map<string, Message>>();
  let current: Map<string, Message> | null = null;
  for (const message of messages) {
    if (message.role === 'assistant') {
      current = new Map();
      steps.set(message.id, current);
    } else if (message.role === 'tool' && message.toolCallId) {
      current?.set(message.toolCallId, message);
    } else {
      current = null;
    }
  }
  return steps;
}

/** A command the agent ran (or runs now), for the terminal panel's Agent tab. */
export type AgentCommand = {
  callId: string;
  command: string;
  /** Saved output; a running command's output arrives live instead. */
  output: string | null;
  exitCode: number | null;
  durationMs: number | null;
  /** `pending`: not answered yet (running, or waiting for permission). */
  status: ToolStatus | 'pending';
};

/** The commands the agent ran in a session, oldest first, each with its answer's output. */
export function agentCommands(messages: Message[]): AgentCommand[] {
  const steps = answersByStep(messages);
  const commands: AgentCommand[] = [];
  for (const message of messages) {
    if (message.role !== 'assistant' || !message.toolCalls) continue;
    const answers = steps.get(message.id);
    for (const call of message.toolCalls) {
      if (canonicalTool(call.function.name) !== 'run_command') continue;
      const answer = answers?.get(call.id);
      const display = answer?.toolDisplay;
      const args = parseArgs(call);
      const command = display?.command ?? args.command ?? args.cmd;
      commands.push({
        callId: call.id,
        command: typeof command === 'string' ? command : '',
        output: display?.output ?? null,
        exitCode: display?.exitCode ?? null,
        durationMs: display?.durationMs ?? null,
        status: answer ? (answer.toolStatus ?? 'ok') : 'pending',
      });
    }
  }
  return commands;
}

/** A page a chat reply drew on. `read`: the model read it, not just its search snippet. */
export type WebSource = { url: string; title: string; read: boolean };

/** The pages a web search reply used: the ones it read first, then search results, each once. */
export function webSources(answers: Message[]): WebSource[] {
  const sources: WebSource[] = [];
  const seen = new Set<string>();
  const add = (url: string | undefined, title: string | undefined, read: boolean) => {
    if (!url) return;
    const key = url.replace(/\/$/, '');
    if (seen.has(key)) return;
    seen.add(key);
    sources.push({ url, title: title || url, read });
  };
  const ok = answers.filter((m) => m.toolStatus === 'ok');
  for (const m of ok) {
    if (m.toolName === 'web_fetch') add(m.toolDisplay?.url, m.toolDisplay?.pageTitle, true);
  }
  for (const m of ok) {
    if (m.toolName === 'web_search') {
      for (const hit of m.toolDisplay?.results ?? []) add(hit.url, hit.title, false);
    }
  }
  return sources;
}

/** A chat reply that searched first: its first step (for versions and regenerating) and sources. */
export type WebTurn = { start: Message; sources: WebSource[] };

/** Chat replies that searched or read before answering, by the id of the answer. */
export function webTurns(messages: Message[]): Map<string, WebTurn> {
  const turns = new Map<string, WebTurn>();
  let start: Message | null = null;
  let answers: Message[] = [];
  for (const message of messages) {
    if (message.role === 'user') {
      start = null;
      answers = [];
    } else if (message.role === 'tool') {
      answers.push(message);
    } else if (message.role === 'assistant') {
      start ??= message;
      if (!message.toolCalls?.length && start !== message) {
        turns.set(message.id, { start, sources: webSources(answers) });
      }
    }
  }
  return turns;
}

/** The newest to-do list in a session. */
export function latestTodos(messages: Message[]): Todo[] {
  for (let i = messages.length - 1; i >= 0; i--) {
    const todos = messages[i]!.toolDisplay?.todos;
    if (todos) return todos;
  }
  return [];
}
