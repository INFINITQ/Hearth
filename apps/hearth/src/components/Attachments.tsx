// Files and images on messages: thumbnails (loaded when shown, larger on click) and file chips.

import { FileText, ImageOff, LoaderCircle, TriangleAlert, X } from 'lucide-react';
import { Dialog } from 'radix-ui';
import { useEffect, useState } from 'react';
import { formatSize, imageCache } from '../lib/attachments';
import { backend } from '../lib/backend';
import { cn } from '../lib/cn';
import type { AttachmentInfo } from '../lib/types';

/** A file as a chip: icon, name and size (or why it can't be attached). */
export function FileChip({
  name,
  detail,
  state = 'ready',
  onRemove,
}: {
  name: string;
  detail: string;
  state?: 'ready' | 'reading' | 'error';
  onRemove?: () => void;
}) {
  return (
    <span
      className={cn(
        'inline-flex h-9 max-w-[260px] items-center gap-2 rounded-[10px] px-2.5 text-[12.5px]',
        state === 'error' ? 'bg-danger-soft text-danger' : 'bg-surface-2 text-text',
      )}
      title={state === 'error' ? detail : `${name} · ${detail}`}
      data-file={name}
    >
      {state === 'reading' ? (
        <LoaderCircle className="size-4 shrink-0 animate-spin text-accent" />
      ) : state === 'error' ? (
        <TriangleAlert className="size-4 shrink-0" />
      ) : (
        <FileText className="size-4 shrink-0 text-text-3" />
      )}
      <span className="min-w-0 leading-tight">
        <span className="block truncate font-medium">{name}</span>
        <span
          className={cn(
            'block truncate text-[11px]',
            state === 'error' ? 'text-danger' : 'text-text-3',
          )}
        >
          {detail}
        </span>
      </span>
      {onRemove && (
        <button
          type="button"
          aria-label={`Remove ${name}`}
          onClick={onRemove}
          className="-mr-1 grid size-5 shrink-0 place-items-center rounded-[5px] text-text-3 hover:bg-surface-3 hover:text-text"
        >
          <X className="size-3" />
        </button>
      )}
    </span>
  );
}

/** An attached image, loaded when it's shown; opens larger. */
function AttachmentImage({ attachment }: { attachment: AttachmentInfo }) {
  const [src, setSrc] = useState<string | null>(() => imageCache.get(attachment.id) ?? null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (src) return;
    let live = true;
    backend()
      .attachmentData(attachment.id)
      .then(({ data }) => {
        if (!live) return;
        if (data) {
          imageCache.set(attachment.id, data);
          setSrc(data);
        } else {
          setFailed(true);
        }
      })
      .catch(() => live && setFailed(true));
    return () => {
      live = false;
    };
  }, [attachment.id, src]);

  if (!src) {
    return (
      <span
        className="grid size-24 place-items-center rounded-[10px] bg-surface-2 text-text-3"
        title={attachment.name}
      >
        {failed ? (
          <ImageOff className="size-5" />
        ) : (
          <LoaderCircle className="size-4 animate-spin" />
        )}
      </span>
    );
  }
  return (
    <Dialog.Root>
      <Dialog.Trigger asChild>
        <button
          type="button"
          aria-label={`Open ${attachment.name}`}
          className="overflow-hidden rounded-[10px] ring-1 ring-line transition-opacity duration-150 hover:opacity-90"
        >
          <img
            src={src}
            alt={attachment.name}
            data-image={attachment.name}
            className="block max-h-40 min-h-16 min-w-16 max-w-[240px] bg-surface-2 object-contain"
          />
        </button>
      </Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="fixed left-1/2 top-1/2 z-50 max-h-[88vh] max-w-[88vw] -translate-x-1/2 -translate-y-1/2 outline-none">
          <Dialog.Title className="sr-only">{attachment.name}</Dialog.Title>
          <img
            src={src}
            alt={attachment.name}
            className="max-h-[88vh] max-w-[88vw] rounded-card object-contain shadow-pop"
          />
          <Dialog.Close asChild>
            <button
              type="button"
              aria-label="Close"
              className="absolute -right-3 -top-3 grid size-8 place-items-center rounded-full bg-surface text-text-2 shadow-pop ring-1 ring-line hover:text-text"
            >
              <X className="size-4" />
            </button>
          </Dialog.Close>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function detailOf(attachment: AttachmentInfo): string {
  if (attachment.mime === 'application/pdf') return `PDF · ${formatSize(attachment.size)}`;
  return formatSize(attachment.size);
}

/** The files and images on a message, above its text. */
export function AttachmentList({ attachments }: { attachments: AttachmentInfo[] }) {
  if (!attachments.length) return null;
  return (
    <div className="mb-1.5 flex max-w-[85%] flex-wrap justify-end gap-1.5" aria-label="Attachments">
      {attachments.map((attachment) =>
        attachment.kind === 'image' ? (
          <AttachmentImage key={attachment.id} attachment={attachment} />
        ) : (
          <FileChip key={attachment.id} name={attachment.name} detail={detailOf(attachment)} />
        ),
      )}
    </div>
  );
}
