// An in-memory stand-in for the conversation backend, used by unit tests and when the UI runs
// in a normal browser (`pnpm dev:web`). It streams a canned reply so every state can be seen.

import type { ChatBackend } from './backend';
import type {
  LocalModel,
  Message,
  ModelStatus,
  PermissionChoice,
  PermissionMode,
  Project,
  RunEnvelope,
  RunEvent,
  RunInfo,
  Session,
  SessionView,
  TerminalEvent,
  TerminalInfo,
  ToolCall,
  ToolDisplay,
  ToolStatus,
} from './types';

const CALC_DIFF = `--- a/src/calc.py
+++ b/src/calc.py
@@ -1,2 +1,2 @@
 def add(a, b):
-    return a - b
+    return a + b
`;

const SAMPLE_REPLY = `Here's a quick overview.

## Big-O at a glance

| Operation | Array | Hash map |
| --- | --- | --- |
| Lookup | O(n) | O(1) |
| Insert | O(1)* | O(1) |

\`\`\`rust
fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}
\`\`\`

The variance is $\\sigma^2 = \\frac{1}{n}\\sum_i (x_i - \\mu)^2$.`;

const GB = 1024 ** 3;

const FAKE_MODELS: LocalModel[] = [
  {
    id: 'm-qwen',
    path: 'D:\\Models\\qwen\\Qwen3.6-35B-A3B-UD-IQ4_XS.gguf',
    fileName: 'Qwen3.6-35B-A3B-UD-IQ4_XS.gguf',
    name: 'Qwen3.6 35B A3B',
    quant: 'UD-IQ4_XS',
    sizeBytes: 17.7 * GB,
    parts: 1,
    missingParts: 0,
    architecture: 'qwen3moe',
    sizeLabel: '35B-A3B',
    contextLength: 262_144,
    layers: 48,
    experts: 128,
    tools: true,
    thinking: true,
    mmproj: 'D:\\Models\\qwen\\mmproj-F16.gguf',
  },
  {
    id: 'm-gemma',
    path: 'D:\\Models\\gemma\\gemma-4-26B-A4B-it-UD-Q4_K_XL.gguf',
    fileName: 'gemma-4-26B-A4B-it-UD-Q4_K_XL.gguf',
    name: 'Gemma 4 26B A4B',
    quant: 'UD-Q4_K_XL',
    sizeBytes: 17 * GB,
    parts: 1,
    missingParts: 0,
    architecture: 'gemma4',
    sizeLabel: '26B-A4B',
    contextLength: 131_072,
    layers: 30,
    experts: 128,
    tools: true,
    thinking: true,
    mmproj: null,
  },
  {
    id: 'm-oss',
    path: 'D:\\Models\\gpt-oss\\gpt-oss-20b-MXFP4.gguf',
    fileName: 'gpt-oss-20b-MXFP4.gguf',
    name: 'gpt-oss-20b',
    quant: 'MXFP4',
    sizeBytes: 12.1 * GB,
    parts: 1,
    missingParts: 0,
    architecture: 'gpt-oss',
    sizeLabel: '20B',
    contextLength: 131_072,
    layers: 24,
    experts: 32,
    tools: true,
    thinking: true,
    mmproj: null,
  },
];

export function fakeChat(): ChatBackend {
  const sessions = new Map<string, Session>();
  const loaded = new Map<string, ModelStatus>();
  const runtimeListeners = new Set<(status: ModelStatus[]) => void>();
  const messages = new Map<string, Message>();
  const runs = new Map<string, { sessionId: string; stop: () => void }>();
  const projects = new Map<string, Project>();
  const waits = new Map<string, (answer: string) => void>();
  const sessionListeners = new Set<() => void>();
  const runListeners = new Set<(info: RunInfo) => void>();
  const terminals = new Map<
    string,
    {
      info: TerminalInfo;
      line: string;
      shown: string;
      listener: ((e: TerminalEvent) => void) | null;
    }
  >();
  const webKeys = { brave: false, tavily: false };
  let counter = 0;
  const newId = () => `fake-${Date.now().toString(36)}-${(counter++).toString(36)}`;
  const changed = () => sessionListeners.forEach((listener) => listener());
  const newSession = (mode: Session['mode'], extra: Partial<Session> = {}): Session => {
    const now = Date.now();
    const session: Session = {
      id: newId(),
      mode,
      title: '',
      titleSource: 'none',
      workspaceId: null,
      modelRef: null,
      params: null,
      persona: null,
      permissionMode: null,
      searchMode: null,
      activeLeafId: null,
      pinned: false,
      archived: false,
      createdAt: now,
      updatedAt: now,
      ...extra,
    };
    sessions.set(session.id, session);
    changed();
    return session;
  };
  const openProject = (path: string): Project => {
    const known = [...projects.values()].find((p) => p.path.toLowerCase() === path.toLowerCase());
    const project: Project = known ?? {
      id: newId(),
      path,
      name: path.split(/[\\/]/).filter(Boolean).at(-1) ?? path,
      trusted: false,
      createdAt: Date.now(),
      lastOpenedAt: 0,
      exists: true,
    };
    project.lastOpenedAt = Date.now() + counter++;
    projects.set(project.id, project);
    return { ...project };
  };

  const siblings = (message: Message) =>
    [...messages.values()]
      .filter(
        (m) =>
          m.sessionId === message.sessionId &&
          m.parentId === message.parentId &&
          m.role === message.role,
      )
      .sort((a, b) => a.createdAt - b.createdAt)
      .map((m) => m.id);

  const pathTo = (id: string | null): Message[] => {
    const out: Message[] = [];
    for (
      let cursor = id ? messages.get(id) : undefined;
      cursor;
      cursor = cursor.parentId ? messages.get(cursor.parentId) : undefined
    ) {
      out.unshift(cursor);
    }
    return out;
  };

  const view = (id: string): SessionView => {
    const session = sessions.get(id);
    if (!session) throw { kind: 'invalid', message: "The conversation doesn't exist any more." };
    const run = [...runs.entries()].find(([, r]) => r.sessionId === id);
    return {
      session,
      messages: pathTo(session.activeLeafId).map((m) => {
        const ids = siblings(m);
        return { ...m, versions: { index: ids.indexOf(m.id), ids } };
      }),
      run: run ? { runId: run[0], sessionId: id, state: 'streaming' } : null,
    };
  };

  const insert = (
    session: Session,
    fields: Partial<Message> & Pick<Message, 'role' | 'parentId'>,
  ): Message => {
    const message: Message = {
      id: newId(),
      sessionId: session.id,
      content: '',
      reasoning: null,
      reasoningMs: null,
      modelLabel: null,
      finishReason: null,
      status: 'complete',
      error: null,
      usage: null,
      createdAt: Date.now() + counter,
      versions: { index: 0, ids: [] },
      ...fields,
    };
    messages.set(message.id, message);
    session.activeLeafId = message.id;
    session.updatedAt = Date.now();
    return message;
  };

  return {
    sessionsList: async (mode) =>
      [...sessions.values()]
        .filter((s) => s.mode === mode && !s.archived)
        .sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.updatedAt - a.updatedAt),
    sessionCreate: async (mode) => newSession(mode),
    sessionGet: async (id) => view(id),
    sessionUpdate: async (id, patch) => {
      const session = sessions.get(id);
      if (!session) throw { kind: 'invalid', message: 'Not found' };
      Object.assign(session, patch);
      if (patch.title !== undefined) session.titleSource = 'user';
      changed();
      return session;
    },
    sessionDelete: async (id) => {
      sessions.delete(id);
      for (const [key, message] of messages) if (message.sessionId === id) messages.delete(key);
      changed();
    },
    branchSelect: async (sessionId, messageId) => {
      const session = sessions.get(sessionId)!;
      let leaf = messages.get(messageId)!;
      for (;;) {
        const children = [...messages.values()]
          .filter((m) => m.parentId === leaf.id)
          .sort((a, b) => b.createdAt - a.createdAt);
        if (!children[0]) break;
        leaf = children[0];
      }
      session.activeLeafId = leaf.id;
      return view(sessionId);
    },
    search: async (query) => {
      const needle = query.trim().toLowerCase();
      if (!needle) return [];
      return [...sessions.values()]
        .filter((s) => s.title.toLowerCase().includes(needle))
        .map((s) => ({
          sessionId: s.id,
          messageId: null,
          mode: s.mode,
          title: s.title,
          snippet: s.title,
          updatedAt: s.updatedAt,
        }));
    },
    sessionExport: async () => null,
    connectionsList: async () => [
      {
        id: 'preview',
        name: 'Preview',
        baseUrl: 'http://127.0.0.1:8080',
        hasApiKey: false,
        llamaExtensions: true,
        createdAt: 0,
      },
    ],
    connectionSave: async (input) => ({
      id: input.id ?? newId(),
      name: input.name,
      baseUrl: input.baseUrl,
      hasApiKey: !!input.apiKey,
      llamaExtensions: input.llamaExtensions,
      createdAt: Date.now(),
    }),
    connectionDelete: async () => {},
    connectionModels: async () => ['preview-model'],
    connectionTest: async () => ['preview-model'],
    runStart: async (sessionId, action, onEvent) => {
      const session = sessions.get(sessionId);
      if (!session) throw { kind: 'invalid', message: 'Not found' };
      const runId = newId();
      let seq = 0;
      const emit = (event: RunEvent) => onEvent({ ...event, runId, seq: ++seq } as RunEnvelope);
      if (session.mode === 'code') {
        if (action.kind !== 'send') throw { kind: 'invalid', message: 'Only new tasks here.' };
        return agentRun(session, action.text, runId, emit);
      }
      let userMessageId: string | null = null;
      let parent: string;
      if (action.kind === 'regenerate') {
        const target = messages.get(action.messageId)!;
        parent = target.role === 'assistant' ? target.parentId! : target.id;
      } else {
        const parentId =
          action.kind === 'edit' ? messages.get(action.messageId)!.parentId : session.activeLeafId;
        const user = insert(session, { role: 'user', parentId, content: action.text.trim() });
        userMessageId = user.id;
        parent = user.id;
      }
      const reply = insert(session, {
        role: 'assistant',
        parentId: parent,
        status: 'streaming',
        modelLabel: 'preview-model',
      });
      const timers: ReturnType<typeof setTimeout>[] = [];
      const finish = (status: 'completed' | 'cancelled') => {
        reply.status = status === 'completed' ? 'complete' : 'interrupted';
        reply.reasoningMs = 1200;
        emit({
          type: 'message_finished',
          messageId: reply.id,
          status: reply.status,
          finishReason: 'stop',
          usage: { promptTokens: 120, completionTokens: 96, predictedPerSecond: 48.2 },
          reasoningMs: 1200,
        });
        if (!session.title) {
          const first = pathTo(reply.id).find((m) => m.role === 'user');
          session.title = first?.content.split('\n')[0]?.slice(0, 60) ?? 'New chat';
          session.titleSource = 'auto';
          emit({ type: 'title_changed', sessionId, title: session.title });
        }
        emit({ type: 'finished', status });
        runs.delete(runId);
        runListeners.forEach((l) => l({ runId, sessionId, state: 'finished' }));
        changed();
      };
      runs.set(runId, {
        sessionId,
        stop: () => {
          timers.forEach(clearTimeout);
          finish('cancelled');
        },
      });
      runListeners.forEach((l) => l({ runId, sessionId, state: 'starting' }));
      setTimeout(() => {
        emit({ type: 'started', sessionId, userMessageId, model: 'preview-model' });
        emit({ type: 'message_started', messageId: reply.id, parentId: parent });
        const reasoning =
          'The user wants an overview. A table, a code sample and one formula will do.';
        const chunks = [
          ...reasoning.match(/.{1,12}/gs)!.map((text) => ({ kind: 'r' as const, text })),
          ...SAMPLE_REPLY.match(/.{1,6}/gs)!.map((text) => ({ kind: 'c' as const, text })),
        ];
        chunks.forEach((chunk, index) => {
          timers.push(
            setTimeout(
              () => {
                if (chunk.kind === 'r') {
                  reply.reasoning = (reply.reasoning ?? '') + chunk.text;
                  emit({ type: 'reasoning_delta', messageId: reply.id, text: chunk.text });
                } else {
                  reply.content += chunk.text;
                  emit({ type: 'content_delta', messageId: reply.id, text: chunk.text });
                }
                if (index === chunks.length - 1) finish('completed');
              },
              30 * (index + 1),
            ),
          );
        });
      }, 0);
      changed();
      return { runId, userMessageId };
    },
    runAttach: async () => false,
    runCancel: async (runId) => {
      runs.get(runId)?.stop();
      return true;
    },
    runsActive: async () =>
      [...runs.entries()].map(([runId, r]) => ({
        runId,
        sessionId: r.sessionId,
        state: 'streaming',
      })),
    onSessionsChanged: (listener) => {
      sessionListeners.add(listener);
      return () => sessionListeners.delete(listener);
    },
    onRunState: (listener) => {
      runListeners.add(listener);
      return () => runListeners.delete(listener);
    },
    openLink: async () => {},

    projectPick: async () => openProject('C:\\Users\\you\\code\\demo-app'),
    projectOpen: async (path) => openProject(path),
    projectsRecent: async () =>
      [...projects.values()].sort((a, b) => b.lastOpenedAt - a.lastOpenedAt).map((p) => ({ ...p })),
    projectTrust: async (id) => {
      const project = projects.get(id);
      if (!project) throw { kind: 'invalid', message: 'Not found' };
      project.trusted = true;
      return { ...project };
    },
    projectSessions: async (projectId) =>
      [...sessions.values()]
        .filter((s) => s.mode === 'code' && s.workspaceId === projectId && !s.archived)
        .sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.updatedAt - a.updatedAt),
    codeSessionCreate: async (projectId, permissionMode) =>
      newSession('code', { workspaceId: projectId, permissionMode }),
    runPermission: async (requestId, decision) => {
      const resolve = waits.get(requestId);
      waits.delete(requestId);
      resolve?.(decision);
      return !!resolve;
    },
    runAnswer: async (questionId, answer) => {
      const resolve = waits.get(questionId);
      waits.delete(questionId);
      resolve?.(answer);
      return !!resolve;
    },
    runSteer: async () => {},
    sessionChanges: async (sessionId) => {
      const edited = [...messages.values()].some(
        (m) => m.sessionId === sessionId && m.toolName === 'edit_file' && m.toolStatus === 'ok',
      );
      return edited
        ? [
            {
              path: 'src/calc.py',
              abs: 'C:\\Users\\you\\code\\demo-app\\src\\calc.py',
              kind: 'modified',
              diff: CALC_DIFF,
              added: 1,
              removed: 1,
              binary: false,
            },
          ]
        : [];
    },
    sessionRevert: async () => {},
    sessionUndo: async () => ['src/calc.py'],
    shellsInfo: async () => ({
      gitBash: 'C:\\Program Files\\Git\\bin\\bash.exe',
      powershell: 'C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe',
    }),

    // A pretend shell: it echoes what you type and answers a few commands.
    terminalOpen: async (shell, projectId) => {
      const info: TerminalInfo = {
        id: newId(),
        shell: shell ?? 'git_bash',
        cwd: (projectId && projects.get(projectId)?.path) || 'C:\\Users\\you',
        projectId,
        running: true,
        exitCode: null,
        createdAt: Date.now(),
      };
      terminals.set(info.id, { info, line: '', shown: '', listener: null });
      printTerminal(info.id, terminalPrompt(info));
      return info;
    },
    terminalAttach: async (id, onEvent) => {
      const terminal = terminals.get(id);
      if (!terminal) throw new Error('This terminal has closed.');
      terminal.listener = onEvent;
      if (terminal.shown) onEvent({ type: 'output', data: terminal.shown });
    },
    terminalWrite: async (id, data) => {
      const terminal = terminals.get(id);
      if (!terminal) return;
      for (const char of data) {
        if (char === '\r') {
          const reply = fakeShellReply(terminal.line.trim());
          terminal.line = '';
          printTerminal(id, `\r\n${reply}${terminalPrompt(terminal.info)}`);
        } else if (char === '\x7f') {
          if (terminal.line) {
            terminal.line = terminal.line.slice(0, -1);
            printTerminal(id, '\b \b');
          }
        } else if (char >= ' ') {
          terminal.line += char;
          printTerminal(id, char);
        }
      }
    },
    terminalResize: async () => {},
    terminalClose: async (id) => {
      terminals.delete(id);
    },
    terminalsList: async () => [...terminals.values()].map((t) => t.info),

    webKeys: async () => ({ ...webKeys }),
    webKeySet: async (provider, key) => {
      webKeys[provider] = key.trim().length > 0;
      return { ...webKeys };
    },
    webTest: async () => [
      {
        title: 'Open-source language models',
        url: 'https://example.com/models',
        snippet: 'A list of open models you can run yourself.',
      },
    ],

    runtimeOverview: async () => ({
      server: {
        path: 'C:\\Tools\\llama.cpp\\llama-server.exe',
        version: 'build 7960 (3f2a1bc)',
        devices: [
          { id: 'CUDA0', name: 'NVIDIA GeForce RTX 5070', totalMib: 12227, freeMib: 11043 },
        ],
        warnings: [],
      },
      serverError: null,
      folders: [{ path: 'D:\\Models', exists: true, automatic: false }],
      models: FAKE_MODELS,
      status: [...loaded.values()],
    }),
    modelLoad: async (modelId) => {
      loaded.set(modelId, { modelId, state: 'loading', sinceMs: Date.now(), port: 0, command: '' });
      runtimeListeners.forEach((l) => l([...loaded.values()]));
      await new Promise((resolve) => setTimeout(resolve, 900));
      for (const other of loaded.keys()) if (other !== modelId) loaded.delete(other);
      loaded.set(modelId, { modelId, state: 'ready', sinceMs: Date.now(), port: 0, command: '' });
      runtimeListeners.forEach((l) => l([...loaded.values()]));
    },
    modelUnload: async (modelId) => {
      loaded.delete(modelId);
      runtimeListeners.forEach((l) => l([...loaded.values()]));
    },
    modelLogs: async () => ['main: server is listening on http://127.0.0.1:52011'],
    runtimeTelemetry: async () => ({
      gpus: [
        { name: 'NVIDIA GeForce RTX 5070', totalMib: 12227, usedMib: loaded.size ? 9800 : 900 },
      ],
      ramTotalMib: 32_607,
      ramUsedMib: loaded.size ? 22_400 : 9_100,
    }),
    pickLlamaServer: async () => 'C:\\Tools\\llama.cpp\\llama-server.exe',
    pickModelFolder: async () => 'D:\\Models',
    onRuntime: (listener) => {
      runtimeListeners.add(listener);
      return () => runtimeListeners.delete(listener);
    },
  };

  function printTerminal(id: string, data: string) {
    const terminal = terminals.get(id);
    if (!terminal) return;
    terminal.shown += data;
    terminal.listener?.({ type: 'output', data });
  }

  /** A scripted agent task: read a file, plan, then edit it (asking first in Ask mode). */
  function agentRun(
    session: Session,
    text: string,
    runId: string,
    emit: (event: RunEvent) => void,
  ) {
    let stopped = false;
    const user = insert(session, { role: 'user', parentId: session.activeLeafId, content: text });
    const pause = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
    const waitFor = (id: string) => new Promise<string>((resolve) => waits.set(id, resolve));

    const step = async (content: string, call?: { name: string; args: object }) => {
      const reply = insert(session, {
        role: 'assistant',
        parentId: session.activeLeafId,
        status: 'streaming',
        modelLabel: 'preview-model',
      });
      emit({ type: 'message_started', messageId: reply.id, parentId: reply.parentId });
      for (const piece of content.match(/.{1,6}/gs) ?? []) {
        await pause(25);
        reply.content += piece;
        emit({ type: 'content_delta', messageId: reply.id, text: piece });
      }
      const toolCalls: ToolCall[] = call
        ? [
            {
              id: newId(),
              type: 'function',
              function: { name: call.name, arguments: JSON.stringify(call.args) },
            },
          ]
        : [];
      reply.status = 'complete';
      reply.toolCalls = toolCalls.length ? toolCalls : null;
      emit({
        type: 'message_finished',
        messageId: reply.id,
        status: 'complete',
        finishReason: call ? 'tool_calls' : 'stop',
        usage: { promptTokens: 900, completionTokens: 40, predictedPerSecond: 41.5 },
        reasoningMs: null,
        toolCalls,
      });
      return toolCalls[0];
    };

    const answer = (call: ToolCall, status: ToolStatus, content: string, display: ToolDisplay) => {
      const message = insert(session, {
        role: 'tool',
        parentId: session.activeLeafId,
        content,
        toolCallId: call.id,
        toolName: call.function.name,
        toolStatus: status,
        toolDisplay: display,
      });
      emit({
        type: 'tool_finished',
        callId: call.id,
        status,
        summary: display.summary ?? '',
        message,
      });
    };

    const script = async () => {
      emit({
        type: 'started',
        sessionId: session.id,
        userMessageId: user.id,
        model: 'preview-model',
      });
      if (!session.title) {
        session.title = text.split('\n')[0]!.slice(0, 60);
        session.titleSource = 'auto';
        emit({ type: 'title_changed', sessionId: session.id, title: session.title });
      }
      const read = await step("I'll look at the code first.", {
        name: 'read_file',
        args: { path: 'src/calc.py' },
      });
      if (stopped || !read) return 'cancelled';
      emit({ type: 'tool_started', callId: read.id, tool: 'read_file', title: 'Read src/calc.py' });
      await pause(300);
      answer(read, 'ok', '     1→def add(a, b):\n     2→    return a - b', {
        title: 'Read src/calc.py',
        summary: '2 lines',
        path: 'src/calc.py',
      });

      const plan = await step('', {
        name: 'todo_write',
        args: {},
      });
      if (stopped || !plan) return 'cancelled';
      const todos = [
        { content: 'Fix the sign in add()', status: 'in_progress' as const },
        { content: 'Explain the change', status: 'pending' as const },
      ];
      emit({
        type: 'tool_started',
        callId: plan.id,
        tool: 'todo_write',
        title: 'Update the to-do list',
      });
      answer(plan, 'ok', 'To-do list updated', {
        title: 'Update the to-do list',
        summary: '0 of 2 done',
        todos,
      });

      const edit = await step('`add` subtracts instead of adding. Fixing that.', {
        name: 'edit_file',
        args: { path: 'src/calc.py', old_string: 'return a - b', new_string: 'return a + b' },
      });
      if (stopped || !edit) return 'cancelled';
      emit({ type: 'tool_started', callId: edit.id, tool: 'edit_file', title: 'Edit src/calc.py' });
      const display: ToolDisplay = {
        title: 'Edit src/calc.py',
        path: 'src/calc.py',
        diff: CALC_DIFF,
      };
      if ((session.permissionMode as PermissionMode | null) === 'ask') {
        const requestId = newId();
        emit({
          type: 'permission_request',
          requestId,
          callId: edit.id,
          tool: 'edit_file',
          class: 'edit',
          title: 'Edit src/calc.py',
          path: 'src/calc.py',
          diff: CALC_DIFF,
          command: null,
          danger: null,
          choices: [
            { id: 'allow_once', label: 'Allow' },
            { id: 'allow_session', label: 'Allow edits this session' },
            { id: 'deny', label: 'Deny' },
          ],
        });
        const decision = (await waitFor(requestId)) as PermissionChoice['id'];
        emit({ type: 'permission_resolved', requestId, decision });
        if (decision === 'deny') {
          answer(edit, 'denied', 'The user declined this edit_file call.', {
            ...display,
            summary: 'Declined',
          });
          await step('Understood: I left `src/calc.py` as it was.');
          return 'completed';
        }
      }
      await pause(200);
      answer(edit, 'ok', 'Edited src/calc.py', { ...display, summary: '' });
      await step('Fixed `add` in `src/calc.py`: it now returns `a + b` instead of `a - b`.');
      return 'completed';
    };

    runs.set(runId, { sessionId: session.id, stop: () => (stopped = true) });
    runListeners.forEach((l) => l({ runId, sessionId: session.id, state: 'starting' }));
    void script().then((status) => {
      emit({ type: 'finished', status: stopped ? 'cancelled' : (status as 'completed') });
      runs.delete(runId);
      runListeners.forEach((l) => l({ runId, sessionId: session.id, state: 'finished' }));
      changed();
    });
    changed();
    return { runId, userMessageId: user.id };
  }
}

function terminalPrompt(info: TerminalInfo): string {
  const folder = info.cwd.split(/[\\/]/).pop() ?? '';
  return info.shell === 'powershell'
    ? `PS ${info.cwd}> `
    : `\x1b[32myou@hearth\x1b[0m \x1b[35mMINGW64\x1b[0m \x1b[33m~/${folder}\x1b[0m\r\n$ `;
}

function fakeShellReply(line: string): string {
  if (!line) return '';
  if (line.startsWith('echo ')) return `${line.slice(5)}\r\n`;
  if (line === 'ls') return 'README.md  src  tests\r\n';
  return `${line.split(' ')[0]}: not available in the browser preview\r\n`;
}
