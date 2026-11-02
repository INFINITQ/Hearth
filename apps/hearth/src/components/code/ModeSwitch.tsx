import { Check, ChevronDown, Hand, PencilLine, Telescope, Zap } from 'lucide-react';
import { DropdownMenu } from 'radix-ui';
import { cn } from '../../lib/cn';
import type { PermissionMode } from '../../lib/types';
import { useChat } from '../../stores/chat';

export const MODES: {
  value: PermissionMode;
  label: string;
  note: string;
  icon: typeof Hand;
}[] = [
  {
    value: 'plan',
    label: 'Plan',
    note: 'Looks around and proposes a plan. Changes nothing.',
    icon: Telescope,
  },
  { value: 'ask', label: 'Ask', note: 'Asks before every change.', icon: Hand },
  {
    value: 'auto_edit',
    label: 'Auto-edit',
    note: 'Edits files in the project; asks before anything else.',
    icon: PencilLine,
  },
  { value: 'full_auto', label: 'Full auto', note: 'No questions inside the project.', icon: Zap },
];

/** The open session's mode, or the one a new session will start with. */
export function useMode(): PermissionMode {
  return useChat((s) =>
    s.session?.mode === 'code' && s.session.permissionMode
      ? (s.session.permissionMode as PermissionMode)
      : s.draftMode,
  );
}

/** Shift+Tab in the message box: the next mode. */
export function cycleMode() {
  const state = useChat.getState();
  const current =
    state.session?.mode === 'code' && state.session.permissionMode
      ? state.session.permissionMode
      : state.draftMode;
  const index = MODES.findIndex((mode) => mode.value === current);
  void state.setPermissionMode(MODES[(index + 1) % MODES.length]!.value);
}

/** How freely the agent may act, in the message box. */
export function ModeSwitch() {
  const mode = useMode();
  const current = MODES.find((m) => m.value === mode) ?? MODES[2]!;
  const Icon = current.icon;
  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger asChild>
        <button
          type="button"
          aria-label={`Permission mode: ${current.label}`}
          title="Permission mode (Shift+Tab)"
          data-mode={mode}
          className={cn(
            'flex h-7 items-center gap-1.5 rounded-full px-2.5 text-[12.5px] font-medium transition-colors duration-150 hover:bg-surface-3/70 data-[state=open]:bg-surface-3',
            mode === 'full_auto' ? 'text-accent-text' : 'text-text-2 hover:text-text',
          )}
        >
          <Icon className="size-3.5 stroke-[2]" />
          {current.label}
          <ChevronDown className="size-3 opacity-70" />
        </button>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content side="top" align="start" sideOffset={6} className="menu w-[300px]">
          {MODES.map((option) => (
            <DropdownMenu.Item
              key={option.value}
              className="menu-item menu-item-tall"
              onSelect={() => void useChat.getState().setPermissionMode(option.value)}
            >
              <option.icon />
              <span className="min-w-0 flex-1">
                <span className="block font-medium text-text">{option.label}</span>
                <span className="block text-[12px] leading-snug text-text-3">{option.note}</span>
              </span>
              <Check className={cn(option.value !== mode && 'invisible')} />
            </DropdownMenu.Item>
          ))}
          <DropdownMenu.Separator className="menu-sep" />
          <p className="menu-note">Shift+Tab in the message box switches modes.</p>
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}
