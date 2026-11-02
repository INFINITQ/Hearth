import { SlidersHorizontal } from 'lucide-react';
import { Popover } from 'radix-ui';
import type { ReactNode } from 'react';
import type { ChatParams } from '../lib/types';
import { useChat } from '../stores/chat';
import { IconButton } from './ui/Button';
import { Segmented } from './ui/Segmented';

type Thinking = 'default' | 'on' | 'off';

/** Per-chat settings: thinking, temperature, reply length and instructions. */
export function ParamsPopover() {
  const session = useChat((s) => s.session);
  const draft = useChat((s) => s.draft);
  const params: ChatParams = (session ? session.params : draft.params) ?? {};
  const persona = session ? session.persona : draft.persona;
  const update = (next: ChatParams, nextPersona?: string | null) =>
    void useChat.getState().setParams(next, nextPersona);
  const thinking: Thinking = params.thinking == null ? 'default' : params.thinking ? 'on' : 'off';
  const temperature = params.temperature ?? null;

  return (
    <Popover.Root>
      <Popover.Trigger asChild>
        <IconButton label="Chat settings">
          <SlidersHorizontal />
        </IconButton>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          side="top"
          align="start"
          sideOffset={8}
          className="popover w-[320px] space-y-4 p-4"
        >
          <Field label="Thinking" hint="For models that can think before answering.">
            <Segmented<Thinking>
              label="Thinking"
              value={thinking}
              onChange={(next) =>
                update({ ...params, thinking: next === 'default' ? null : next === 'on' })
              }
              options={[
                { value: 'default', label: 'Default' },
                { value: 'on', label: 'On' },
                { value: 'off', label: 'Off' },
              ]}
            />
          </Field>
          <Field
            label={`Temperature${temperature === null ? '' : ` · ${temperature.toFixed(2)}`}`}
            hint="Lower is focused and repeatable; higher is more varied."
            action={
              temperature !== null && (
                <button
                  type="button"
                  className="text-[12px] text-accent-text hover:underline"
                  onClick={() => update({ ...params, temperature: null })}
                >
                  Reset
                </button>
              )
            }
          >
            <input
              type="range"
              aria-label="Temperature"
              min={0}
              max={2}
              step={0.05}
              value={temperature ?? 0.7}
              onChange={(e) => update({ ...params, temperature: Number(e.target.value) })}
              className="w-full accent-[var(--accent)]"
            />
          </Field>
          <Field label="Longest reply" hint="In tokens. Empty means no limit.">
            <input
              type="number"
              min={16}
              step={64}
              placeholder="No limit"
              aria-label="Longest reply"
              defaultValue={params.maxTokens ?? ''}
              onBlur={(e) =>
                update({ ...params, maxTokens: e.target.value ? Number(e.target.value) : null })
              }
              className="h-8 w-full rounded-control bg-surface-2 px-2.5 text-[13px] outline-none ring-1 ring-line focus:ring-accent select-text"
            />
          </Field>
          <Field label="Instructions for this chat" hint="Replaces the default system prompt.">
            <textarea
              rows={3}
              aria-label="Instructions for this chat"
              placeholder="e.g. Answer like a patient teacher."
              defaultValue={persona ?? ''}
              onBlur={(e) => update(params, e.target.value.trim() || null)}
              className="w-full resize-none rounded-control bg-surface-2 px-2.5 py-2 text-[13px] leading-relaxed outline-none ring-1 ring-line focus:ring-accent select-text"
            />
          </Field>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}

function Field({
  label,
  hint,
  action,
  children,
}: {
  label: string;
  hint?: string;
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div>
      <div className="mb-1.5 flex items-center justify-between">
        <span className="text-[13px] font-medium">{label}</span>
        {action}
      </div>
      {children}
      {hint && <p className="mt-1 text-[12px] text-text-3">{hint}</p>}
    </div>
  );
}
