import { ChevronRight, RotateCcw, X } from 'lucide-react';
import { AlertDialog } from 'radix-ui';
import { useState } from 'react';
import { cn } from '../../lib/cn';
import type { FileChange } from '../../lib/types';
import { useChat } from '../../stores/chat';
import { Button, IconButton } from '../ui/Button';
import { DiffView } from './DiffView';

const KIND: Record<FileChange['kind'], { letter: string; label: string; className: string }> = {
  added: { letter: 'A', label: 'Added', className: 'text-[var(--diff-add-mark)]' },
  modified: { letter: 'M', label: 'Modified', className: 'text-accent-text' },
  deleted: { letter: 'D', label: 'Deleted', className: 'text-[var(--diff-del-mark)]' },
};

/** The files the agent changed in this session, each with its diff and a way back. */
export function ChangesPanel() {
  const changes = useChat((s) => s.changes);
  return (
    <aside
      aria-label="Changes"
      className="flex w-[400px] shrink-0 flex-col border-l border-line bg-bg"
    >
      <div className="flex h-11 shrink-0 items-center gap-2 pl-4 pr-2">
        <h2 className="text-[13.5px] font-semibold">Changes</h2>
        {changes && changes.length > 0 && (
          <span className="text-[12.5px] text-text-3">
            {changes.length === 1 ? '1 file' : `${changes.length} files`}
          </span>
        )}
        <div className="flex-1" />
        <IconButton label="Close changes" onClick={() => useChat.getState().toggleChanges(false)}>
          <X />
        </IconButton>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        {changes === null && <p className="px-2 py-2 text-[13px] text-text-3">Loading…</p>}
        {changes?.length === 0 && (
          <p className="px-2 py-2 text-[13px] text-text-3">
            The agent hasn't changed any files yet.
          </p>
        )}
        {changes?.map((change) => (
          <ChangeRow key={change.abs} change={change} />
        ))}
      </div>
      <p className="shrink-0 border-t border-line px-4 py-2.5 text-[11.5px] leading-snug text-text-3">
        Compared with each file before this session. Changes made by commands (builds, formatters,
        git) aren't tracked here.
      </p>
    </aside>
  );
}

function ChangeRow({ change }: { change: FileChange }) {
  const [open, setOpen] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const kind = KIND[change.kind];
  const name = change.path.split('/').at(-1) ?? change.path;
  const folder = change.path.slice(0, change.path.length - name.length).replace(/\/$/, '');
  return (
    <div className="group" data-change={change.kind}>
      <div className="flex items-center rounded-control transition-colors duration-150 hover:bg-surface-2">
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
          className="flex h-9 min-w-0 flex-1 items-center gap-2.5 pl-2 text-left text-[13px]"
        >
          <span
            className={cn('w-3 shrink-0 font-mono text-[11.5px] font-semibold', kind.className)}
            title={kind.label}
          >
            {kind.letter}
          </span>
          <span className="min-w-0 truncate" title={change.path}>
            <span className="font-medium text-text">{name}</span>
            {folder && <span className="ml-1.5 text-[12px] text-text-3">{folder}</span>}
          </span>
          <span className="ml-auto shrink-0 font-mono text-[11.5px] tabular-nums">
            {change.added > 0 && (
              <span className="text-[var(--diff-add-mark)]">+{change.added}</span>
            )}{' '}
            {change.removed > 0 && (
              <span className="text-[var(--diff-del-mark)]">−{change.removed}</span>
            )}
          </span>
          <ChevronRight
            className={cn(
              'size-3.5 shrink-0 text-text-3 transition-transform duration-200',
              open && 'rotate-90',
            )}
          />
        </button>
        <IconButton
          label={`Revert ${change.path}`}
          className="mr-1 opacity-0 focus-visible:opacity-100 group-hover:opacity-100"
          onClick={() => setConfirming(true)}
        >
          <RotateCcw />
        </IconButton>
      </div>
      {open && (
        <div className="mb-2 mt-1 pl-2 pr-1">
          {change.binary ? (
            <p className="px-1 text-[12.5px] text-text-3">A binary file: no text to compare.</p>
          ) : (
            <DiffView diff={change.diff} />
          )}
        </div>
      )}
      <AlertDialog.Root open={confirming} onOpenChange={setConfirming}>
        <AlertDialog.Portal>
          <AlertDialog.Overlay className="dialog-overlay" />
          <AlertDialog.Content className="dialog w-[400px]">
            <AlertDialog.Title className="text-[15px] font-semibold">
              Revert {name}?
            </AlertDialog.Title>
            <AlertDialog.Description className="mt-1.5 text-[13.5px] leading-relaxed text-text-2">
              {change.kind === 'added'
                ? 'The agent created this file in this session, so reverting deletes it.'
                : 'The file goes back to how it was before this session. The agent’s changes to it are lost.'}
            </AlertDialog.Description>
            <div className="mt-5 flex justify-end gap-2">
              <AlertDialog.Cancel asChild>
                <Button variant="ghost">Cancel</Button>
              </AlertDialog.Cancel>
              <AlertDialog.Action asChild>
                <Button variant="danger" onClick={() => void useChat.getState().revertFile(change)}>
                  Revert
                </Button>
              </AlertDialog.Action>
            </div>
          </AlertDialog.Content>
        </AlertDialog.Portal>
      </AlertDialog.Root>
    </div>
  );
}
