import { CornerDownLeft, ShieldQuestionMark, TriangleAlert } from 'lucide-react';
import { useState } from 'react';
import type { PermissionRequest } from '../../lib/types';
import { useChat } from '../../stores/chat';
import { Button } from '../ui/Button';
import { DiffView } from './DiffView';

const HEADINGS: Record<PermissionRequest['class'], string> = {
  edit: 'Allow this change?',
  read: 'Allow reading outside the project?',
  execute: 'Allow this command?',
  network: 'Allow this web request?',
  external: 'Allow this tool?',
  internal: 'Allow this?',
};

/** The agent asks before doing something: the change it wants to make, and the answers. */
export function PermissionCard({ request }: { request: PermissionRequest }) {
  const [note, setNote] = useState('');
  const [sent, setSent] = useState(false);
  const answer = (decision: PermissionRequest['choices'][number]['id'], withNote?: string) => {
    setSent(true);
    void useChat.getState().answerPermission(decision, withNote);
  };
  const heading = HEADINGS[request.class];

  return (
    <section
      aria-label="Permission request"
      className="rounded-card bg-surface p-4 shadow-float ring-1 ring-[color-mix(in_srgb,var(--accent)_45%,transparent)]"
    >
      <div className="flex items-start gap-3">
        <div className="grid size-8 shrink-0 place-items-center rounded-full bg-accent-soft text-accent">
          <ShieldQuestionMark className="size-4" />
        </div>
        <div className="min-w-0 flex-1 pt-0.5">
          <p className="text-[14px] font-semibold text-text">{heading}</p>
          <p className="mt-0.5 truncate text-[13px] text-text-2" title={request.title}>
            {request.title}
          </p>
        </div>
      </div>
      {request.diff && <DiffView diff={request.diff} className="mt-3" />}
      {request.command && (
        <pre
          aria-label="Command"
          className="mt-3 max-h-[180px] overflow-auto whitespace-pre-wrap break-words rounded-[10px] bg-surface-2 px-3 py-2 font-mono text-[12.5px] leading-[1.55] text-text select-text"
        >
          <span className="text-text-3 select-none">$ </span>
          {request.command}
        </pre>
      )}
      {request.danger && (
        <p
          role="alert"
          className="mt-2.5 flex items-start gap-2 rounded-control bg-danger-soft px-3 py-2 text-[12.5px] text-danger"
        >
          <TriangleAlert className="mt-0.5 size-3.5 shrink-0" />
          <span>
            Careful: this command {request.danger}. Hearth always asks before commands like this.
          </span>
        </p>
      )}
      <div className="mt-3.5 flex flex-wrap items-center gap-2">
        {request.choices.map((choice) => (
          <Button
            key={choice.id}
            disabled={sent}
            variant={
              choice.id === 'allow_once' ? 'primary' : choice.id === 'deny' ? 'ghost' : 'secondary'
            }
            onClick={() => answer(choice.id)}
          >
            {choice.label}
          </Button>
        ))}
      </div>
      <form
        className="mt-2.5 flex h-9 items-center gap-2 rounded-control bg-surface-2 pl-3 pr-1.5"
        onSubmit={(event) => {
          event.preventDefault();
          if (note.trim()) answer('deny', note.trim());
        }}
      >
        <input
          value={note}
          disabled={sent}
          onChange={(event) => setNote(event.target.value)}
          aria-label="Tell the agent what to do instead"
          placeholder="Or tell the agent what to do instead…"
          className="min-w-0 flex-1 bg-transparent text-[13px] text-text outline-none select-text placeholder:text-text-3"
        />
        <button
          type="submit"
          aria-label="Deny with this note"
          disabled={sent || !note.trim()}
          className="grid size-6 place-items-center rounded-[6px] text-text-3 transition-colors hover:bg-surface-3 hover:text-text disabled:opacity-40"
        >
          <CornerDownLeft className="size-3.5" />
        </button>
      </form>
    </section>
  );
}
