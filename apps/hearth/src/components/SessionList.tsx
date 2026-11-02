import { FileDown, LoaderCircle, MoreHorizontal, Pencil, Pin, PinOff, Trash2 } from 'lucide-react';
import { AlertDialog, DropdownMenu } from 'radix-ui';
import { useState } from 'react';
import { backend } from '../lib/backend';
import { cn } from '../lib/cn';
import { groupByDate } from '../lib/dates';
import type { Mode, SearchHit, SessionSummary } from '../lib/types';
import { useApp } from '../stores/app';
import { useChat } from '../stores/chat';
import { Button } from './ui/Button';

function openSession(mode: Mode, id: string) {
  useApp.getState().go(mode);
  void useChat.getState().open(id);
}

/** The sidebar's history: pinned first, then by date. While searching, the matches instead. */
export function SessionList({ mode }: { mode: Mode }) {
  const list = useChat((s) => s.lists[mode]);
  const current = useChat((s) => s.session?.id);
  const busy = useChat((s) => s.busy);
  const waiting = useChat((s) => s.waiting);
  const query = useChat((s) => s.query);
  const hits = useChat((s) => s.hits);
  const hasProject = useChat((s) => s.project !== null);

  if (query.trim()) return <SearchResults hits={hits} />;
  if (!list.length) {
    return (
      <p className="px-2.5 py-1.5 text-[13px] text-text-3">
        {mode === 'chat'
          ? 'No conversations yet'
          : hasProject
            ? 'No sessions in this project yet'
            : 'Open a project to see its sessions'}
      </p>
    );
  }
  return (
    <>
      {groupByDate(list, Date.now()).map((group) => (
        <div key={group.label} className="mb-2">
          <p className="px-2.5 pb-1 pt-2 text-[11.5px] font-medium text-text-3">{group.label}</p>
          {group.items.map((summary) => (
            <SessionRow
              key={summary.id}
              summary={summary}
              active={summary.id === current}
              busy={!!busy[summary.id]}
              waiting={!!waiting[summary.id]}
            />
          ))}
        </div>
      ))}
    </>
  );
}

function SessionRow({
  summary,
  active,
  busy,
  waiting,
}: {
  summary: SessionSummary;
  active: boolean;
  busy: boolean;
  waiting: boolean;
}) {
  const [renaming, setRenaming] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const title = summary.title || (summary.mode === 'code' ? 'New session' : 'New chat');

  return (
    <div
      className={cn(
        'group relative flex h-8 items-center rounded-control transition-colors duration-150',
        active ? 'bg-surface-3' : 'hover:bg-surface-3/60',
      )}
    >
      {renaming ? (
        <input
          autoFocus
          aria-label="Conversation name"
          defaultValue={summary.title}
          onBlur={(e) => {
            setRenaming(false);
            const next = e.target.value.trim();
            if (next && next !== summary.title) void useChat.getState().rename(summary.id, next);
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter') e.currentTarget.blur();
            if (e.key === 'Escape') setRenaming(false);
          }}
          className="mx-1 h-7 min-w-0 flex-1 rounded-[6px] bg-surface px-2 text-[13px] outline-none ring-1 ring-accent select-text"
        />
      ) : (
        <button
          type="button"
          title={title}
          aria-current={active ? 'page' : undefined}
          onClick={() => openSession(summary.mode, summary.id)}
          className={cn(
            'flex h-full min-w-0 flex-1 items-center gap-1.5 px-2.5 text-left text-[13px]',
            active ? 'text-text' : 'text-text-2 group-hover:text-text',
          )}
        >
          {summary.pinned && <Pin className="size-3 shrink-0 text-text-3" />}
          <span className="truncate">{title}</span>
        </button>
      )}
      {waiting ? (
        <span
          className="mr-2.5 size-2 shrink-0 rounded-full bg-accent"
          role="img"
          aria-label="Needs your answer"
          title="Needs your answer"
        />
      ) : (
        busy && (
          <LoaderCircle
            className="mr-2 size-3.5 shrink-0 animate-spin text-accent"
            aria-label="Writing"
          />
        )
      )}
      <DropdownMenu.Root>
        <DropdownMenu.Trigger asChild>
          <button
            type="button"
            aria-label={`Options for ${title}`}
            className="mr-1 grid size-6 shrink-0 place-items-center rounded-[6px] text-text-3 opacity-0 transition-opacity hover:bg-surface-3 hover:text-text focus-visible:opacity-100 group-hover:opacity-100 data-[state=open]:opacity-100"
          >
            <MoreHorizontal className="size-4" />
          </button>
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content align="start" sideOffset={4} className="menu w-[190px]">
            <DropdownMenu.Item className="menu-item" onSelect={() => setRenaming(true)}>
              <Pencil /> Rename
            </DropdownMenu.Item>
            <DropdownMenu.Item
              className="menu-item"
              onSelect={() => void useChat.getState().setPinned(summary.id, !summary.pinned)}
            >
              {summary.pinned ? <PinOff /> : <Pin />} {summary.pinned ? 'Unpin' : 'Pin'}
            </DropdownMenu.Item>
            <DropdownMenu.Item
              className="menu-item"
              onSelect={() => void backend().sessionExport(summary.id, 'md')}
            >
              <FileDown /> Export as Markdown
            </DropdownMenu.Item>
            <DropdownMenu.Separator className="menu-sep" />
            <DropdownMenu.Item
              className="menu-item menu-danger"
              onSelect={() => setConfirming(true)}
            >
              <Trash2 /> Delete
            </DropdownMenu.Item>
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>
      <AlertDialog.Root open={confirming} onOpenChange={setConfirming}>
        <AlertDialog.Portal>
          <AlertDialog.Overlay className="dialog-overlay" />
          <AlertDialog.Content className="dialog w-[380px]">
            <AlertDialog.Title className="text-[15px] font-semibold">
              Delete this conversation?
            </AlertDialog.Title>
            <AlertDialog.Description className="mt-1.5 text-[13.5px] text-text-2">
              “{title}” and all its messages will be removed from this PC.
            </AlertDialog.Description>
            <div className="mt-5 flex justify-end gap-2">
              <AlertDialog.Cancel asChild>
                <Button variant="ghost">Cancel</Button>
              </AlertDialog.Cancel>
              <AlertDialog.Action asChild>
                <Button variant="danger" onClick={() => void useChat.getState().remove(summary.id)}>
                  Delete
                </Button>
              </AlertDialog.Action>
            </div>
          </AlertDialog.Content>
        </AlertDialog.Portal>
      </AlertDialog.Root>
    </div>
  );
}

/** Splits a snippet on the \u0002 … \u0003 match markers from SQLite. */
function Highlighted({ text }: { text: string }) {
  const parts: string[] = [];
  let part = '';
  for (const char of text) {
    if (char === '\u0002' || char === '\u0003') {
      parts.push(part);
      part = '';
    } else {
      part += char;
    }
  }
  parts.push(part);
  return (
    <>
      {parts.map((part, index) =>
        index % 2 === 1 ? (
          <mark key={index} className="rounded-[3px] bg-accent-soft px-0.5 text-text">
            {part}
          </mark>
        ) : (
          <span key={index}>{part}</span>
        ),
      )}
    </>
  );
}

function SearchResults({ hits }: { hits: SearchHit[] }) {
  if (!hits.length) return <p className="px-2.5 py-1.5 text-[13px] text-text-3">No matches</p>;
  return (
    <div aria-label="Search results">
      {hits.map((hit) => (
        <button
          key={`${hit.sessionId}-${hit.messageId ?? 'title'}`}
          type="button"
          onClick={() => openSession(hit.mode, hit.sessionId)}
          className="block w-full rounded-control px-2.5 py-2 text-left hover:bg-surface-3/60"
        >
          <span className="block truncate text-[13px] text-text">{hit.title || 'New chat'}</span>
          {hit.messageId && (
            <span className="mt-0.5 line-clamp-2 block text-[12px] leading-snug text-text-3">
              <Highlighted text={hit.snippet} />
            </span>
          )}
        </button>
      ))}
    </div>
  );
}
