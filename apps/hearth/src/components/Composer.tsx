import { ArrowUp, Globe, Paperclip } from 'lucide-react';
import { useLayoutEffect, useRef, useState } from 'react';
import { cn } from '../lib/cn';
import { IconButton } from './ui/Button';

type ComposerProps = {
  placeholder?: string;
  /** Sending is off (no model yet, or a reply is running). Typing still works. */
  disabled?: boolean;
  onSend?(text: string): void;
  className?: string;
};

const MAX_HEIGHT = 240;

/** The message box: grows with its text, Enter sends, Shift+Enter adds a line. */
export function Composer({
  placeholder = 'Ask anything…',
  disabled,
  onSend,
  className,
}: ComposerProps) {
  const [text, setText] = useState('');
  const box = useRef<HTMLTextAreaElement>(null);
  const canSend = !disabled && text.trim().length > 0;

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = '0px';
    el.style.height = `${Math.min(el.scrollHeight, MAX_HEIGHT)}px`;
  }, [text]);

  const send = () => {
    if (!canSend) return;
    onSend?.(text);
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
        aria-label="Message"
        placeholder={placeholder}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
            event.preventDefault();
            send();
          }
        }}
        className="block w-full resize-none bg-transparent px-4 pb-1 pt-3.5 text-[15px] leading-[1.55] text-text outline-none select-text placeholder:text-text-3"
        style={{ maxHeight: MAX_HEIGHT }}
      />
      <div className="flex items-center gap-1 px-2.5 pb-2.5 pt-1">
        <IconButton label="Attach files" disabled>
          <Paperclip />
        </IconButton>
        <IconButton label="Search the web" disabled>
          <Globe />
        </IconButton>
        <div className="flex-1" />
        <button
          type="button"
          aria-label="Send"
          disabled={!canSend}
          onClick={send}
          className="grid size-8 place-items-center rounded-full bg-accent text-on-accent transition-colors duration-150 hover:bg-accent-hover disabled:bg-surface-3 disabled:text-text-3"
        >
          <ArrowUp className="size-[17px] stroke-[2.25]" />
        </button>
      </div>
    </div>
  );
}
