import { useMemo } from 'react';
import { cn } from '../../lib/cn';
import { parseDiff } from '../../lib/diff';

/** A unified diff: removed lines red, added lines green, a gap mark between hunks. */
export function DiffView({ diff, className }: { diff: string; className?: string }) {
  const rows = useMemo(() => parseDiff(diff), [diff]);
  if (!rows.length) {
    return <p className="px-3 py-2 text-[12.5px] text-text-3">No changes to the text.</p>;
  }
  return (
    <div
      className={cn(
        'diff max-h-[340px] overflow-auto rounded-[10px] bg-surface py-1.5 ring-1 ring-line',
        className,
      )}
      aria-label="Changes"
    >
      {rows.map((row, index) =>
        row.kind === 'gap' ? (
          <div key={index} className="diff-gap" aria-hidden>
            ⋯
          </div>
        ) : (
          <div
            key={index}
            className={cn(
              'diff-line',
              row.kind === 'add' && 'diff-add',
              row.kind === 'del' && 'diff-del',
            )}
            data-kind={row.kind}
          >
            <span>{row.line}</span>
            <span>{row.kind === 'add' ? '+' : row.kind === 'del' ? '−' : ''}</span>
            <code>{row.text || ' '}</code>
          </div>
        ),
      )}
    </div>
  );
}
