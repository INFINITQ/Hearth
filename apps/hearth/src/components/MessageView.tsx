import {
  Check,
  ChevronLeft,
  ChevronRight,
  Copy,
  LoaderCircle,
  Pencil,
  RotateCcw,
  TriangleAlert,
} from 'lucide-react';
import { useLayoutEffect, useRef, useState } from 'react';
import { cn } from '../lib/cn';
import type { Message } from '../lib/types';
import { useChat } from '../stores/chat';
import { SummaryDivider } from './ContextMeter';
import { Markdown } from './Markdown';
import { ThinkingBlock } from './ThinkingBlock';
import { Button, IconButton } from './ui/Button';

export function MessageView({
  message,
  live,
  last,
}: {
  message: Message;
  live: boolean;
  last: boolean;
}) {
  if (message.role === 'summary') return <SummaryDivider message={message} />;
  return message.role === 'user' ? (
    <UserMessage message={message} />
  ) : (
    <AssistantMessage message={message} live={live} last={last} />
  );
}

export function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <IconButton
      label={copied ? 'Copied' : 'Copy'}
      onClick={() => {
        void navigator.clipboard.writeText(text).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1500);
        });
      }}
    >
      {copied ? <Check /> : <Copy />}
    </IconButton>
  );
}

/** ‹ 2 / 3 › between versions of a message (edits or regenerations). */
function Versions({ message }: { message: Message }) {
  const busy = useChat((s) => s.run !== null);
  const { index, ids } = message.versions;
  if (ids.length < 2) return null;
  const show = (step: -1 | 1) => void useChat.getState().showVersion(message, step);
  return (
    <div
      className="mr-1 flex items-center text-[12px] tabular-nums text-text-3"
      aria-label="Versions"
    >
      <IconButton label="Previous version" disabled={busy || index === 0} onClick={() => show(-1)}>
        <ChevronLeft />
      </IconButton>
      <span>
        {index + 1} / {ids.length}
      </span>
      <IconButton
        label="Next version"
        disabled={busy || index === ids.length - 1}
        onClick={() => show(1)}
      >
        <ChevronRight />
      </IconButton>
    </div>
  );
}

function UserMessage({ message }: { message: Message }) {
  const [editing, setEditing] = useState(false);
  const busy = useChat((s) => s.run !== null);
  if (editing) return <EditBox message={message} onDone={() => setEditing(false)} />;
  return (
    <div className="group flex flex-col items-end" data-role="user">
      <div className="max-w-[85%] whitespace-pre-wrap break-words rounded-[18px] bg-surface-2 px-4 py-2.5 text-[15px] leading-[1.6] select-text">
        {message.content}
      </div>
      <div className="mt-1 flex items-center">
        <Versions message={message} />
        <div className="flex items-center opacity-0 transition-opacity duration-150 focus-within:opacity-100 group-hover:opacity-100">
          <CopyButton text={message.content} />
          <IconButton label="Edit" disabled={busy} onClick={() => setEditing(true)}>
            <Pencil />
          </IconButton>
        </div>
      </div>
    </div>
  );
}

function EditBox({ message, onDone }: { message: Message; onDone(): void }) {
  const [text, setText] = useState(message.content);
  const box = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = '0px';
    el.style.height = `${Math.min(el.scrollHeight, 320)}px`;
  }, [text]);
  const save = () => {
    if (!text.trim()) return;
    onDone();
    void useChat.getState().edit(message.id, text);
  };
  return (
    <div className="ml-auto w-[85%] rounded-card bg-surface p-3 shadow-float ring-1 ring-line-strong">
      <textarea
        ref={box}
        autoFocus
        aria-label="Edit message"
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
            e.preventDefault();
            save();
          } else if (e.key === 'Escape') {
            onDone();
          }
        }}
        className="block w-full resize-none bg-transparent text-[15px] leading-[1.6] outline-none select-text"
      />
      <div className="mt-2 flex justify-end gap-2">
        <Button size="sm" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button size="sm" variant="primary" disabled={!text.trim()} onClick={save}>
          Send
        </Button>
      </div>
    </div>
  );
}

/** Model · speed · tokens, under a finished reply. */
export function stats(message: Message): string {
  const parts: string[] = [];
  if (message.modelLabel) parts.push(message.modelLabel);
  const speed = message.usage?.predictedPerSecond;
  if (speed) parts.push(`${speed.toFixed(speed < 10 ? 1 : 0)} tok/s`);
  if (message.usage?.completionTokens)
    parts.push(`${message.usage.completionTokens.toLocaleString()} tokens`);
  return parts.join(' · ');
}

function AssistantMessage({
  message,
  live,
  last,
}: {
  message: Message;
  live: boolean;
  last: boolean;
}) {
  const busy = useChat((s) => s.run !== null);
  const loadingModel = useChat((s) => (live ? s.run?.loading : null));
  const streaming = live && message.status === 'streaming';
  const reasoning = message.reasoning ?? '';
  const info = stats(message);
  return (
    <div className="group" data-role="assistant" data-status={message.status}>
      {reasoning && (
        <ThinkingBlock
          text={reasoning}
          ms={message.reasoningMs}
          live={streaming && !message.content}
        />
      )}
      {message.content ? (
        <Markdown text={message.content} streaming={streaming} />
      ) : (
        streaming &&
        !reasoning &&
        (loadingModel ? <ModelLoadingLine model={loadingModel} /> : <TypingDots />)
      )}
      {message.status === 'error' && (
        <div className="mt-2 flex items-start gap-2 rounded-card bg-danger-soft px-3.5 py-2.5 text-[13.5px] text-danger">
          <TriangleAlert className="mt-0.5 size-4 shrink-0" />
          <span className="min-w-0 flex-1 select-text">{message.error ?? 'The reply failed.'}</span>
        </div>
      )}
      {message.status === 'interrupted' && (
        <p className="mt-2 text-[12.5px] text-text-3">Stopped</p>
      )}
      {!streaming && (
        <div
          className={cn(
            'mt-1.5 flex items-center transition-opacity duration-150',
            last ? 'opacity-100' : 'opacity-0 focus-within:opacity-100 group-hover:opacity-100',
          )}
        >
          <Versions message={message} />
          {message.content && <CopyButton text={message.content} />}
          <IconButton
            label={message.status === 'error' ? 'Try again' : 'Regenerate'}
            disabled={busy}
            onClick={() => void useChat.getState().regenerate(message.id)}
          >
            <RotateCcw />
          </IconButton>
          {info && <span className="ml-2 truncate text-[12px] text-text-3">{info}</span>}
        </div>
      )}
    </div>
  );
}

/** A spinner and what Hearth is doing before the reply starts. */
export function ProgressLine({ text }: { text: string }) {
  return (
    <p className="flex items-center gap-2 text-[13px] text-text-2" aria-live="polite">
      <LoaderCircle className="size-3.5 animate-spin text-accent" />
      <span className="thinking-shimmer">{text}</span>
    </p>
  );
}

/** Shown while Hearth starts the model's llama-server. */
export function ModelLoadingLine({ model }: { model: string }) {
  return <ProgressLine text={`Loading ${model}… the first reply can take a minute.`} />;
}

export function TypingDots() {
  return (
    <div className="typing-dots flex h-6 items-center gap-1" aria-label="Writing">
      <span />
      <span />
      <span />
    </div>
  );
}
