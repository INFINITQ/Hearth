import { ArrowUp, Paperclip, Square } from 'lucide-react';
import { type ReactNode, useLayoutEffect, useRef, useState } from 'react';
import { cn } from '../lib/cn';
import { IconButton } from './ui/Button';

type ComposerProps = {
  placeholder?: string;
  /** A reply is being written: the send button becomes Stop (Esc works too). */
  running?: boolean;
  onSend(text: string): void;
  onStop?(): void;
  /** Extra buttons next to Attach (chat settings…). */
  tools?: ReactNode;
  /** Shift+Tab in the box (coding sessions: the next permission mode). */
  onShiftTab?(): void;
  /** Messages can be sent while running (coding sessions: they join the task). */
  sendWhileRunning?: boolean;
  autoFocus?: boolean;
  className?: string;
};

const MAX_HEIGHT = 240;

/** The message box: grows with its text, Enter sends, Shift+Enter adds a line. */
export function Composer({
  placeholder = 'Ask anything…',
  running = false,
  onSend,
  onStop,
  tools,
  onShiftTab,
  sendWhileRunning = false,
  autoFocus,
  className,
}: ComposerProps) {
  const [text, setText] = useState('');
  const box = useRef<HTMLTextAreaElement>(null);
  const canSend = (!running || sendWhileRunning) && text.trim().length > 0;

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = '0px';
    el.style.height = `${Math.min(el.scrollHeight, MAX_HEIGHT)}px`;
  }, [text]);

  const send = () => {
    if (!canSend) return;
    onSend(text.trim());
    setText('');
  };

  return (
    <div
      className={cn(
        'rounded-card bg-surface shadow-float ring-1 ring-line transition-shadow duration-150 focus-within:ring-line-strong',
        className,
      )}
    >
      <textarea
        ref={box}
        rows={1}
        value={text}
        autoFocus={autoFocus}
        aria-label="Message"
        placeholder={placeholder}
        onChange={(event) => setText(event.target.value)}
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
        <IconButton label="Attach files (coming soon)" disabled>
          <Paperclip />
        </IconButton>
        {tools}
        <div className="flex-1" />
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
