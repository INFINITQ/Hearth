// The chat store against the in-memory backend: what reaches the screen while a run goes on.

import { afterEach, describe, expect, it, vi } from 'vitest';
import { backend } from '../lib/backend';
import { useChat } from './chat';

afterEach(async () => {
  useChat.getState().stop();
  await vi.waitFor(() => expect(useChat.getState().run).toBeNull());
});

describe('starting a coding task', () => {
  it('keeps steps that finish while the transcript is being read', async () => {
    const fake = backend();
    // Reads return what was saved when they were asked, but arrive late: the run's first step
    // finishes (with its read_file call) in between.
    const read = fake.sessionGet.bind(fake);
    vi.spyOn(fake, 'sessionGet').mockImplementation(async (id) => {
      const view = structuredClone(await read(id));
      await new Promise((resolve) => setTimeout(resolve, 400));
      return view;
    });
    const project = await fake.projectOpen('C:\\Projects\\calc');
    useChat.setState({ project });

    await useChat.getState().send('Fix add()', 'code');

    const { messages, run } = useChat.getState();
    expect(run).not.toBeNull();
    // The saved user message took the place of the one shown while sending…
    expect(messages[0]?.id.startsWith('pending-')).toBe(false);
    expect(messages[0]?.content).toBe('Fix add()');
    // …and the step that finished during the read is still there, with its call.
    const step = messages.find((m) => m.toolCalls?.[0]?.function.name === 'read_file');
    expect(step?.content).toBe("I'll look at the code first.");
  });
});
