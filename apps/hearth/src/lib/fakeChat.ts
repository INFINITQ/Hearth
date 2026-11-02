// An in-memory stand-in for the conversation backend, used by unit tests and when the UI runs
// in a normal browser (`pnpm dev:web`). It streams a canned reply so every state can be seen.

import type { ChatBackend } from './backend';
import type { Message, RunEnvelope, RunEvent, RunInfo, Session, SessionView } from './types';

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

export function fakeChat(): ChatBackend {
  const sessions = new Map<string, Session>();
  const messages = new Map<string, Message>();
  const runs = new Map<string, { sessionId: string; stop: () => void }>();
  const sessionListeners = new Set<() => void>();
  const runListeners = new Set<(info: RunInfo) => void>();
  let counter = 0;
  const newId = () => `fake-${Date.now().toString(36)}-${(counter++).toString(36)}`;
  const changed = () => sessionListeners.forEach((listener) => listener());

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
    sessionCreate: async (mode) => {
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
      };
      sessions.set(session.id, session);
      changed();
      return session;
    },
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
  };
}
