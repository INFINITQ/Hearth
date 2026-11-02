// Web search in chats: the Web button, the searches and pages a reply went through, and the
// sources under its answer.

import { ChevronRight, Globe, LoaderCircle, Search, TriangleAlert } from 'lucide-react';
import { memo, useState } from 'react';
import { backend } from '../lib/backend';
import { cn } from '../lib/cn';
import { type WebSource, parseArgs, sameAnswers } from '../lib/toolCalls';
import type { Message, ToolCall } from '../lib/types';
import { useChat } from '../stores/chat';
import { Markdown } from './Markdown';
import { ThinkingBlock } from './ThinkingBlock';

function host(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, '');
  } catch {
    return url;
  }
}

function open(url: string) {
  void backend()
    .openLink(url)
    .catch(() => {});
}

/** The composer's Web button: replies may search the web and read pages. */
export function WebToggle() {
  const on = useChat((s) => s.web);
  return (
    <button
      type="button"
      aria-pressed={on}
      aria-label="Search the web"
      title={on ? 'Web search is on: replies can search and read pages' : 'Search the web'}
      onClick={() => useChat.getState().setWeb(!on)}
      className="flex h-7 items-center gap-1.5 rounded-full px-2.5 text-[12.5px] font-medium text-text-2 transition-colors duration-150 hover:bg-surface-3/70 hover:text-text aria-pressed:bg-accent-soft aria-pressed:text-accent-text"
    >
      <Globe className="size-[15px] stroke-[1.75]" />
      Web
    </button>
  );
}

type WebStepsProps = {
  message: Message;
  answers: Map<string, Message> | undefined;
  /** Right after another step: they read as one list. */
  follows?: boolean;
};

/**
 * A step where the model searched or read pages: one line each, opening to the results.
 * Memoized like chat messages (the answers map is rebuilt on every change, so it's compared
 * by contents).
 */
export const WebSteps = memo(
  function WebSteps({ message, answers, follows = false }: WebStepsProps) {
    const running = useChat((s) => s.run?.tool?.callId ?? null);
    const reasoning = message.reasoning ?? '';
    return (
      <div className={cn('space-y-1', follows && '-mt-6')} data-role="web-step">
        {reasoning && <ThinkingBlock text={reasoning} ms={message.reasoningMs} live={false} />}
        {message.content.trim() && <Markdown text={message.content} streaming={false} />}
        {(message.toolCalls ?? []).map((call) => (
          <WebCall
            key={call.id}
            call={call}
            answer={answers?.get(call.id)}
            running={running === call.id}
          />
        ))}
      </div>
    );
  },
  (prev, next) =>
    prev.message === next.message &&
    prev.follows === next.follows &&
    sameAnswers(prev.answers, next.answers),
);

function WebCall({
  call,
  answer,
  running,
}: {
  call: ToolCall;
  answer: Message | undefined;
  running: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const display = answer?.toolDisplay;
  const args = parseArgs(call);
  const search = (answer?.toolName ?? call.function.name).includes('search');
  const failed = answer?.toolStatus === 'error';
  const query = display?.query ?? (typeof args.query === 'string' ? args.query : '');
  const url = display?.url ?? (typeof args.url === 'string' ? args.url : '');
  let text: string;
  if (search) {
    text = answer ? `Searched “${query}”` : `Searching the web for “${query}”…`;
  } else if (!answer) {
    text = `Reading ${host(url)}…`;
  } else {
    text = failed ? `Couldn’t read ${host(url)}` : `Read ${display?.pageTitle ?? host(url)}`;
  }
  const results = display?.results ?? [];
  const canOpen = (search && results.length > 0) || failed;
  const Icon = failed ? TriangleAlert : search ? Search : Globe;
  return (
    <div data-web={search ? 'search' : 'read'} data-status={answer?.toolStatus ?? 'running'}>
      <button
        type="button"
        aria-expanded={canOpen ? expanded : undefined}
        disabled={!canOpen}
        onClick={() => setExpanded(!expanded)}
        className="group flex max-w-full items-center gap-2 py-0.5 text-left text-[13px] text-text-2 enabled:hover:text-text"
      >
        {!answer && running ? (
          <LoaderCircle className="size-3.5 shrink-0 animate-spin text-accent" />
        ) : (
          <Icon className={cn('size-3.5 shrink-0', failed ? 'text-danger' : 'text-text-3')} />
        )}
        <span className={cn('min-w-0 truncate', !answer && 'thinking-shimmer')}>{text}</span>
        {search && answer && display?.summary && !failed && (
          <span className="shrink-0 text-text-3">· {display.summary}</span>
        )}
        {!search && answer && !failed && (
          <span className="shrink-0 truncate text-text-3">· {host(url)}</span>
        )}
        {canOpen && (
          <ChevronRight
            className={cn(
              'size-3.5 shrink-0 text-text-3 transition-transform duration-150',
              expanded && 'rotate-90',
            )}
          />
        )}
      </button>
      {expanded && failed && (
        <p className="ml-5.5 mt-1 text-[12.5px] text-text-2 select-text">{answer?.content}</p>
      )}
      {expanded && !failed && (
        <ol className="ml-5.5 mt-1 space-y-1.5 border-l border-line pl-3">
          {results.map((hit) => (
            <li key={hit.url} className="text-[12.5px] leading-snug">
              <button
                type="button"
                onClick={() => open(hit.url)}
                className="text-left font-medium text-text hover:underline"
              >
                {hit.title}
              </button>
              <span className="ml-1.5 text-text-3">{host(hit.url)}</span>
              {hit.snippet && <p className="text-text-2">{hit.snippet}</p>}
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}

/** The pages an answer drew on, as chips that open in the browser. */
export function Sources({ sources }: { sources: WebSource[] }) {
  const [all, setAll] = useState(false);
  const shown = all ? sources : sources.slice(0, 4);
  return (
    <div className="mt-3 flex flex-wrap items-center gap-1.5" role="list" aria-label="Sources">
      {shown.map((source) => (
        <button
          key={source.url}
          type="button"
          role="listitem"
          title={`${source.title}\n${source.url}`}
          onClick={() => open(source.url)}
          className="flex h-7 max-w-[240px] items-center gap-1.5 rounded-full bg-surface-2 px-2.5 text-[12px] transition-colors duration-150 hover:bg-surface-3"
        >
          {source.read ? (
            <Globe className="size-3 shrink-0 text-accent" />
          ) : (
            <Search className="size-3 shrink-0 text-text-3" />
          )}
          <span className="shrink-0 font-medium text-text">{host(source.url)}</span>
          <span className="min-w-0 truncate text-text-3">{source.title}</span>
        </button>
      ))}
      {!all && sources.length > shown.length && (
        <button
          type="button"
          onClick={() => setAll(true)}
          className="h-7 rounded-full px-2.5 text-[12px] text-text-2 hover:bg-surface-3/70 hover:text-text"
        >
          +{sources.length - shown.length} more
        </button>
      )}
    </div>
  );
}
