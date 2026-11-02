import { ArrowDown, TriangleAlert, X } from 'lucide-react';
import { useLayoutEffect, useRef, useState } from 'react';
import { useApp } from '../stores/app';
import { useChat } from '../stores/chat';
import type { LiveRun } from '../lib/runReducer';
import { MessageView, ModelLoadingLine, ProgressLine, TypingDots } from './MessageView';
import { Button, IconButton } from './ui/Button';

/** The conversation: follows new text unless you scroll up to read. */
export function Transcript() {
  const messages = useChat((s) => s.messages);
  const run = useChat((s) => s.run);
  const box = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const [away, setAway] = useState(false);

  useLayoutEffect(() => {
    const el = box.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [messages, run?.progress, run?.compacting, run?.loading]);

  const onScroll = () => {
    const el = box.current;
    if (!el) return;
    const near = el.scrollHeight - el.scrollTop - el.clientHeight < 120;
    stick.current = near;
    setAway(!near);
  };

  return (
    <div className="relative min-h-0 flex-1">
      <div
        ref={box}
        onScroll={onScroll}
        className="h-full overflow-y-auto"
        role="log"
        aria-label="Conversation"
      >
        <div className="mx-auto max-w-[760px] space-y-7 px-6 pb-10 pt-8">
          {messages.map((message, index) => (
            <MessageView
              key={message.id}
              message={message}
              live={run?.messageId === message.id}
              last={index === messages.length - 1}
            />
          ))}
          {run && <BeforeReply run={run} />}
          <RunErrorBanner />
        </div>
      </div>
      {away && (
        <button
          type="button"
          aria-label="Jump to latest"
          onClick={() => {
            stick.current = true;
            box.current?.scrollTo({ top: box.current.scrollHeight, behavior: 'smooth' });
          }}
          className="absolute bottom-3 left-1/2 grid size-8 -translate-x-1/2 place-items-center rounded-full bg-surface text-text-2 shadow-pop ring-1 ring-line hover:text-text"
        >
          <ArrowDown className="size-4" />
        </button>
      )}
    </div>
  );
}

/** What happens before the reply's first words: loading the model, summarizing, reading. */
function BeforeReply({ run }: { run: LiveRun }) {
  if (run.compacting) {
    return <ProgressLine text="Summarizing the earlier conversation to make room…" />;
  }
  if (run.status !== 'starting') return null;
  if (run.loading) return <ModelLoadingLine model={run.loading} />;
  return <Waiting progress={run.progress} />;
}

export function Waiting({ progress }: { progress: { processed: number; total: number } | null }) {
  if (progress && progress.total > 0) {
    const percent = Math.min(100, Math.round((progress.processed / progress.total) * 100));
    return <p className="text-[13px] text-text-3">Reading the conversation… {percent}%</p>;
  }
  return <TypingDots />;
}

/** Problems starting a reply (no model chosen, server down): shown where the reply would be. */
export function RunErrorBanner() {
  const error = useChat((s) => s.error);
  if (!error) return null;
  const needsModel = error.message.startsWith('Choose a model');
  return (
    <div
      role="alert"
      className="flex items-start gap-2.5 rounded-card bg-danger-soft px-4 py-3 text-[13.5px] text-danger"
    >
      <TriangleAlert className="mt-0.5 size-4 shrink-0" />
      <div className="min-w-0 flex-1">
        <p className="select-text">{error.message}</p>
        {needsModel && (
          <Button
            size="sm"
            variant="secondary"
            className="mt-2"
            onClick={() => useApp.getState().go('settings')}
          >
            Open settings
          </Button>
        )}
      </div>
      <IconButton
        label="Dismiss"
        className="text-danger hover:text-danger"
        onClick={() => useChat.getState().dismissError()}
      >
        <X />
      </IconButton>
    </div>
  );
}
