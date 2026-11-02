import { ChevronRight, Layers } from 'lucide-react';
import { useState } from 'react';
import { cn } from '../lib/cn';
import type { Message } from '../lib/types';
import { useChat } from '../stores/chat';
import { Markdown } from './Markdown';

function tokens(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(n >= 10_000 ? 0 : 1)}K` : String(n);
}

/** How full the model's context window is, after the newest reply. */
export function ContextMeter() {
  const messages = useChat((s) => s.messages);
  const live = useChat((s) => s.run?.context ?? null);
  let used: number | null = live?.used ?? null;
  let limit: number | null = live?.limit ?? null;
  if (!live) {
    for (let i = messages.length - 1; i >= 0; i--) {
      const message = messages[i]!;
      // Older counts don't apply once the conversation was summarized.
      if (message.role === 'summary') break;
      const usage = message.usage;
      if (message.role === 'assistant' && usage?.contextWindow && usage.promptTokens) {
        used = usage.promptTokens + usage.completionTokens;
        limit = usage.contextWindow;
        break;
      }
    }
  }
  if (!used || !limit) return null;
  const share = Math.min(1, used / limit);
  const radius = 6;
  const circumference = 2 * Math.PI * radius;
  return (
    <div
      role="meter"
      aria-label="Context used"
      aria-valuenow={Math.round(share * 100)}
      aria-valuemin={0}
      aria-valuemax={100}
      title={`${tokens(used)} of ${tokens(limit)} tokens in use. Hearth summarizes older messages at 80%.`}
      className="mr-1.5 flex items-center gap-1.5 text-[11.5px] tabular-nums text-text-3"
    >
      <svg viewBox="0 0 16 16" className="size-4 -rotate-90" aria-hidden>
        <circle cx="8" cy="8" r={radius} fill="none" stroke="var(--surface-3)" strokeWidth="2" />
        <circle
          cx="8"
          cy="8"
          r={radius}
          fill="none"
          stroke={share > 0.8 ? 'var(--danger)' : 'var(--accent)'}
          strokeWidth="2"
          strokeDasharray={`${share * circumference} ${circumference}`}
          strokeLinecap="round"
        />
      </svg>
      {Math.round(share * 100)}%
    </div>
  );
}

/** Where earlier messages were summarized; opens to show the summary. */
export function SummaryDivider({ message }: { message: Message }) {
  const [open, setOpen] = useState(false);
  return (
    <div data-role="summary">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="group flex w-full items-center gap-3 text-[12px] text-text-3 hover:text-text-2"
      >
        <span className="h-px flex-1 bg-line" />
        <span className="flex items-center gap-1.5">
          <Layers className="size-3.5" />
          Earlier messages were summarized to make room
          <ChevronRight
            className={cn('size-3.5 transition-transform duration-200', open && 'rotate-90')}
          />
        </span>
        <span className="h-px flex-1 bg-line" />
      </button>
      {open && (
        <div className="mt-2 rounded-card bg-surface-2 px-4 py-3 text-[13.5px]">
          <Markdown text={message.content} streaming={false} />
        </div>
      )}
    </div>
  );
}
