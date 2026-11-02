import { ChevronRight, Sparkles } from 'lucide-react';
import { useState } from 'react';
import { cn } from '../lib/cn';
import { formatDuration } from '../lib/dates';

/** The model's reasoning, collapsed behind a one-line summary. */
export function ThinkingBlock({
  text,
  ms,
  live,
}: {
  text: string;
  ms: number | null;
  live: boolean;
}) {
  const [open, setOpen] = useState(false);
  const label = live ? 'Thinking' : ms != null ? `Thought for ${formatDuration(ms)}` : 'Thoughts';
  return (
    <div className="mb-3">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="inline-flex items-center gap-1.5 rounded-control py-1 text-[13px] text-text-2 transition-colors duration-150 hover:text-text"
      >
        <Sparkles className={cn('size-3.5', live ? 'animate-pulse text-accent' : 'text-text-3')} />
        <span className={cn(live && 'thinking-shimmer')}>{label}</span>
        <ChevronRight
          className={cn('size-3.5 transition-transform duration-200', open && 'rotate-90')}
        />
      </button>
      {open && (
        <div className="mt-1.5 max-h-[320px] overflow-y-auto whitespace-pre-wrap border-l-2 border-line pl-4 text-[13.5px] leading-[1.65] text-text-2 select-text">
          {text}
        </div>
      )}
    </div>
  );
}
