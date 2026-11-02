// Long conversations: while a reply streams, only that reply renders again. The thinking block
// stands in for each message's content, so its renders show which messages rendered.

import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { applyRunEvent, newRun } from '../lib/runReducer';
import type { Message, RunEnvelope } from '../lib/types';
import { useChat } from '../stores/chat';
import { CodeTranscript } from './code/CodeTranscript';
import { Transcript } from './Transcript';

const { rendered } = vi.hoisted(() => ({ rendered: [] as string[] }));

vi.mock('./ThinkingBlock', () => ({
  ThinkingBlock: ({ text }: { text: string }) => {
    rendered.push(text);
    return null;
  },
}));
vi.mock('./Markdown', () => ({ Markdown: ({ text }: { text: string }) => <p>{text}</p> }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function message(id: string, role: Message['role'], extra: Partial<Message> = {}): Message {
  return {
    id,
    sessionId: 's',
    parentId: null,
    role,
    content: `${id} text`,
    reasoning: role === 'assistant' ? `${id} thinking` : null,
    reasoningMs: 500,
    modelLabel: 'model',
    finishReason: 'stop',
    status: 'complete',
    error: null,
    usage: null,
    createdAt: 0,
    versions: { index: 0, ids: [id] },
    ...extra,
  };
}

/** Twenty finished exchanges, then a reply being written. */
function longConversation(): Message[] {
  const history = Array.from({ length: 20 }, (_, i) => [
    message(`u${i}`, 'user', { turn: i + 1 }),
    message(`a${i}`, 'assistant', { turn: i + 1 }),
  ]).flat();
  const live = message('live', 'assistant', {
    content: 'Hello',
    reasoning: 'live thinking',
    status: 'streaming',
  });
  return [...history, live];
}

function streamTokens(count: number) {
  for (let i = 0; i < count; i++) {
    act(() => {
      const s = useChat.getState();
      const event = {
        type: 'content_delta',
        messageId: 'live',
        text: ' more',
        runId: 'r1',
        seq: i + 1,
      } as RunEnvelope;
      useChat.setState(
        applyRunEvent(
          { messages: s.messages, run: s.run, error: s.error, notice: s.notice },
          event,
        ),
      );
    });
  }
}

describe('a long conversation while a reply streams', () => {
  let unmount = () => {};

  afterEach(() => {
    unmount();
    rendered.length = 0;
    useChat.setState({ messages: [], run: null });
  });

  for (const [name, View] of [
    ['chat', Transcript],
    ['coding session', CodeTranscript],
  ] as const) {
    it(`renders only the reply being written again (${name})`, () => {
      useChat.setState({
        messages: longConversation(),
        run: { ...newRun('s', 'r1'), messageId: 'live', status: 'streaming' },
      });
      const root = createRoot(document.body.appendChild(document.createElement('div')));
      unmount = () => act(() => root.unmount());
      act(() => root.render(<View />));
      expect(rendered.filter((text) => text !== 'live thinking')).toHaveLength(20);

      rendered.length = 0;
      streamTokens(5);
      expect(rendered.length).toBeGreaterThan(0);
      expect(rendered.every((text) => text === 'live thinking')).toBe(true);
      expect(useChat.getState().messages.at(-1)?.content).toBe('Hello more more more more more');
    });
  }
});
