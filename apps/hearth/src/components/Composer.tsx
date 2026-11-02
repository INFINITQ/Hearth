import { ArrowUp, Paperclip, Square, X } from 'lucide-react';
import { type ReactNode, useLayoutEffect, useRef, useState } from 'react';
import { type FileKind, MAX_FILES, formatSize, kindOf, readAttachment } from '../lib/attachments';
import { errorMessage } from '../lib/backend';
import { cn } from '../lib/cn';
import type { NewAttachment } from '../lib/types';
import { FileChip } from './Attachments';
import { IconButton } from './ui/Button';

type ComposerProps = {
  placeholder?: string;
  /** A reply is being written: the send button becomes Stop (Esc works too). */
  running?: boolean;
  onSend(text: string, attachments: NewAttachment[]): void;
  onStop?(): void;
  /** Extra buttons next to Attach (chat settings…). */
  tools?: ReactNode;
  /** Shift+Tab in the box (coding sessions: the next permission mode). */
  onShiftTab?(): void;
  /** Messages can be sent while running (coding sessions: they join the task). */
  sendWhileRunning?: boolean;
  /** Shown left of the send button (the context meter). */
  status?: ReactNode;
  autoFocus?: boolean;
  className?: string;
};

/** A file on its way into a message. */
type Pending = {
  key: string;
  name: string;
  size: number;
  kind: FileKind;
  state: 'reading' | 'ready' | 'error';
  attachment?: NewAttachment;
  error?: string;
};

const MAX_HEIGHT = 240;
let keys = 0;

/**
 * The message box: grows with its text, Enter sends, Shift+Enter adds a line. Files can be
 * attached with the paperclip, dropped on it, or pasted.
 */
export function Composer({
  placeholder = 'Ask anything…',
  running = false,
  onSend,
  onStop,
  tools,
  onShiftTab,
  sendWhileRunning = false,
  status,
  autoFocus,
  className,
}: ComposerProps) {
  const [text, setText] = useState('');
  const [files, setFiles] = useState<Pending[]>([]);
  const [dragging, setDragging] = useState(false);
  const box = useRef<HTMLTextAreaElement>(null);
  const picker = useRef<HTMLInputElement>(null);
  const reading = files.some((f) => f.state === 'reading');
  const ready = files.filter((f) => f.state === 'ready' && f.attachment);
  // Messages sent while the agent works join the task as text only.
  const attaching = !running;
  const canSend =
    (!running || sendWhileRunning) && !reading && (text.trim().length > 0 || ready.length > 0);

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = '0px';
    el.style.height = `${Math.min(el.scrollHeight, MAX_HEIGHT)}px`;
  }, [text]);

  const update = (key: string, patch: Partial<Pending>) =>
    setFiles((now) => now.map((f) => (f.key === key ? { ...f, ...patch } : f)));

  const add = (list: FileList | File[] | null) => {
    const incoming = Array.from(list ?? []).slice(0, Math.max(0, MAX_FILES - files.length));
    if (!incoming.length || !attaching) return;
    const pending: Pending[] = incoming.map((file) => ({
      key: `file-${++keys}`,
      name: file.name || 'pasted image',
      size: file.size,
      kind: kindOf(file.name, file.type) ?? 'text',
      state: 'reading',
    }));
    setFiles((now) => [...now, ...pending]);
    incoming.forEach((file, index) => {
      const key = pending[index]!.key;
      readAttachment(file)
        .then((attachment) => update(key, { state: 'ready', attachment }))
        .catch((err: unknown) => update(key, { state: 'error', error: errorMessage(err) }));
    });
  };

  const send = () => {
    if (!canSend) return;
    onSend(text.trim(), running ? [] : ready.map((f) => f.attachment!));
    setText('');
    if (!running) setFiles([]);
  };

  return (
    <div
      className={cn(
        'rounded-card bg-surface shadow-float ring-1 ring-line transition-shadow duration-150 focus-within:ring-line-strong',
        dragging && 'ring-2 ring-accent focus-within:ring-accent',
        className,
      )}
      onDragOver={(event) => {
        if (!attaching || !event.dataTransfer.types.includes('Files')) return;
        event.preventDefault();
        setDragging(true);
      }}
      onDragLeave={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDragging(false);
      }}
      onDrop={(event) => {
        if (!event.dataTransfer.files.length) return;
        event.preventDefault();
        setDragging(false);
        add(event.dataTransfer.files);
      }}
    >
      {files.length > 0 && (
        <div className="flex flex-wrap gap-1.5 px-3 pt-3" aria-label="Files to send">
          {files.map((file) =>
            file.kind === 'image' && file.attachment?.data ? (
              <span key={file.key} className="relative" data-file={file.name}>
                <img
                  src={file.attachment.data}
                  alt={file.name}
                  className="size-12 rounded-[8px] object-cover ring-1 ring-line"
                />
                <button
                  type="button"
                  aria-label={`Remove ${file.name}`}
                  onClick={() => setFiles((now) => now.filter((f) => f.key !== file.key))}
                  className="absolute -right-1.5 -top-1.5 grid size-5 place-items-center rounded-full bg-surface text-text-2 shadow-pop ring-1 ring-line hover:text-text"
                >
                  <X className="size-3" />
                </button>
              </span>
            ) : (
              <FileChip
                key={file.key}
                name={file.name}
                state={file.state}
                detail={
                  file.state === 'error'
                    ? (file.error ?? 'Couldn’t read it')
                    : file.state === 'reading'
                      ? 'Reading…'
                      : file.kind === 'pdf'
                        ? `PDF · ${formatSize(file.size)}`
                        : formatSize(file.size)
                }
                onRemove={() => setFiles((now) => now.filter((f) => f.key !== file.key))}
              />
            ),
          )}
        </div>
      )}
      <textarea
        ref={box}
        rows={1}
        value={text}
        autoFocus={autoFocus}
        aria-label="Message"
        placeholder={placeholder}
        onChange={(event) => setText(event.target.value)}
        onPaste={(event) => {
          if (attaching && event.clipboardData.files.length > 0) {
            event.preventDefault();
            add(event.clipboardData.files);
          }
        }}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
            event.preventDefault();
            send();
          } else if (event.key === 'Escape' && running) {
            event.preventDefault();
            onStop?.();
          } else if (event.key === 'Tab' && event.shiftKey && onShiftTab) {
            event.preventDefault();
            onShiftTab();
          }
        }}
        className="block w-full resize-none bg-transparent px-4 pb-1 pt-3.5 text-[15px] leading-[1.55] text-text outline-none select-text placeholder:text-text-3"
        style={{ maxHeight: MAX_HEIGHT }}
      />
      <div className="flex items-center gap-1 px-2.5 pb-2.5 pt-1">
        <input
          ref={picker}
          type="file"
          multiple
          hidden
          aria-label="Files to attach"
          onChange={(event) => {
            add(event.target.files);
            event.target.value = '';
          }}
        />
        <IconButton
          label="Attach files"
          disabled={!attaching || files.length >= MAX_FILES}
          onClick={() => picker.current?.click()}
        >
          <Paperclip />
        </IconButton>
        {tools}
        <div className="flex-1" />
        {status}
        {running ? (
          <>
            {canSend && (
              <button
                type="button"
                aria-label="Send"
                title="Send now: it joins the task before the next step"
                onClick={send}
                className="grid size-8 place-items-center rounded-full bg-accent text-on-accent transition-colors duration-150 hover:bg-accent-hover"
              >
                <ArrowUp className="size-[17px] stroke-[2.25]" />
              </button>
            )}
            <button
              type="button"
              aria-label="Stop"
              title="Stop (Esc)"
              onClick={onStop}
              className="grid size-8 place-items-center rounded-full bg-text text-bg transition-opacity duration-150 hover:opacity-85"
            >
              <Square className="size-3 fill-current" />
            </button>
          </>
        ) : (
          <button
            type="button"
            aria-label="Send"
            disabled={!canSend}
            onClick={send}
            className="grid size-8 place-items-center rounded-full bg-accent text-on-accent transition-colors duration-150 hover:bg-accent-hover disabled:bg-surface-3 disabled:text-text-3"
          >
            <ArrowUp className="size-[17px] stroke-[2.25]" />
          </button>
        )}
      </div>
    </div>
  );
}
