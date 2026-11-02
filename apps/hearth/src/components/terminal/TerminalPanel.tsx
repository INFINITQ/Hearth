// The terminal panel under a coding session: the Agent tab (the output of the agent's commands)
// and the project's shells, each in a tab.

import { Bot, ChevronDown, Plus, SquareTerminal, TriangleAlert, X } from 'lucide-react';
import { DropdownMenu } from 'radix-ui';
import {
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode,
  Suspense,
  lazy,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { backend } from '../../lib/backend';
import { cn } from '../../lib/cn';
import { type AgentCommand, agentCommands } from '../../lib/toolCalls';
import type { Project, ShellKind, ShellsInfo, TerminalInfo } from '../../lib/types';
import { useChat } from '../../stores/chat';
import { AGENT_TAB, MIN_HEIGHT, useTerminal } from '../../stores/terminal';
import { IconButton } from '../ui/Button';

const TerminalView = lazy(() => import('./TerminalView'));

const SHELL_NAMES: Record<ShellKind, string> = { git_bash: 'Git Bash', powershell: 'PowerShell' };

/** "Git Bash", "Git Bash 2", "PowerShell"… */
function tabNames(terminals: TerminalInfo[]): Map<string, string> {
  const counts = new Map<ShellKind, number>();
  const names = new Map<string, string>();
  for (const terminal of terminals) {
    const count = (counts.get(terminal.shell) ?? 0) + 1;
    counts.set(terminal.shell, count);
    const name = SHELL_NAMES[terminal.shell];
    names.set(terminal.id, count === 1 ? name : `${name} ${count}`);
  }
  return names;
}

export function TerminalPanel({ project }: { project: Project }) {
  const height = useTerminal((s) => s.height);
  const all = useTerminal((s) => s.terminals);
  const active = useTerminal((s) => s.active);
  const error = useTerminal((s) => s.error);
  const agentBusy = useChat((s) => s.run !== null);
  const terminals = useMemo(() => all.filter((t) => t.projectId === project.id), [all, project.id]);
  const names = useMemo(() => tabNames(terminals), [terminals]);
  const shown = terminals.some((t) => t.id === active) ? active : AGENT_TAB;

  return (
    <section
      aria-label="Terminal"
      className="relative flex shrink-0 flex-col border-t border-line bg-surface"
      style={{ height }}
    >
      <ResizeHandle />
      <div
        role="tablist"
        aria-label="Terminals"
        className="flex h-9 shrink-0 items-center gap-0.5 px-2 pt-1"
      >
        <Tab id={AGENT_TAB} label="Agent" selected={shown === AGENT_TAB} live={agentBusy}>
          <Bot />
        </Tab>
        {terminals.map((terminal) => (
          <Tab
            key={terminal.id}
            id={terminal.id}
            label={names.get(terminal.id) ?? ''}
            selected={shown === terminal.id}
            ended={!terminal.running}
            onClose={() => void useTerminal.getState().close(terminal.id)}
          >
            <SquareTerminal />
          </Tab>
        ))}
        <NewTerminal />
        <div className="flex-1" />
        <IconButton
          label="Hide the terminal"
          shortcut="Ctrl+`"
          onClick={() => useTerminal.getState().setOpen(false)}
        >
          <ChevronDown />
        </IconButton>
      </div>
      {error && (
        <div
          role="alert"
          className="mx-3 mt-1 flex items-center gap-2 rounded-control bg-danger-soft px-3 py-1.5 text-[12.5px] text-danger"
        >
          <TriangleAlert className="size-3.5 shrink-0" />
          <span className="min-w-0 flex-1">{error}</span>
          <IconButton
            label="Dismiss"
            className="size-5 text-danger hover:text-danger"
            onClick={() => useTerminal.getState().dismissError()}
          >
            <X />
          </IconButton>
        </div>
      )}
      <div className="relative min-h-0 flex-1">
        <div
          role="tabpanel"
          aria-label="Agent"
          className={cn('absolute inset-0', shown !== AGENT_TAB && 'invisible')}
        >
          <AgentOutput />
        </div>
        {terminals.map((terminal) => (
          <div
            key={terminal.id}
            role="tabpanel"
            aria-label={names.get(terminal.id)}
            className={cn(
              'absolute inset-0 pb-1 pl-3.5 pr-1 pt-1',
              shown !== terminal.id && 'invisible',
            )}
          >
            <Suspense fallback={null}>
              <TerminalView info={terminal} active={shown === terminal.id} />
            </Suspense>
          </div>
        ))}
      </div>
    </section>
  );
}

function Tab({
  id,
  label,
  selected,
  ended = false,
  live = false,
  onClose,
  children,
}: {
  id: string;
  label: string;
  selected: boolean;
  ended?: boolean;
  live?: boolean;
  onClose?: () => void;
  children: ReactNode;
}) {
  return (
    <div
      className={cn(
        'group flex h-7 items-center rounded-control text-[12.5px] transition-colors duration-150',
        selected ? 'bg-surface-3 text-text' : 'text-text-2 hover:bg-surface-3/60 hover:text-text',
      )}
    >
      <button
        type="button"
        role="tab"
        aria-selected={selected}
        onClick={() => useTerminal.getState().select(id)}
        className={cn(
          'flex h-full items-center gap-1.5 pl-2.5 font-medium [&_svg]:size-3.5 [&_svg]:stroke-[1.75]',
          onClose ? 'pr-1' : 'pr-2.5',
        )}
      >
        {children}
        <span className={cn(ended && 'text-text-3 line-through decoration-text-3/60')}>
          {label}
        </span>
        {live && (
          <span
            className="size-1.5 animate-pulse rounded-full bg-accent"
            title="The agent is working"
          />
        )}
      </button>
      {onClose && (
        <button
          type="button"
          aria-label={`Close ${label}`}
          title="Close (ends what runs in it)"
          onClick={onClose}
          className={cn(
            'mr-1 grid size-5 place-items-center rounded-[5px] text-text-3 transition-opacity duration-150 hover:bg-surface-3 hover:text-text focus-visible:opacity-100',
            selected ? 'opacity-100' : 'opacity-0 group-hover:opacity-100',
          )}
        >
          <X className="size-3" />
        </button>
      )}
    </div>
  );
}

/** "+": a new shell of either kind. */
function NewTerminal() {
  const [shells, setShells] = useState<ShellsInfo | null>(null);
  useEffect(() => {
    void backend()
      .shellsInfo()
      .then(setShells)
      .catch(() => {});
  }, []);
  const create = (shell: ShellKind) => void useTerminal.getState().create(shell);
  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger asChild>
        <IconButton label="New terminal">
          <Plus />
        </IconButton>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content align="start" sideOffset={4} className="menu w-[190px]">
          <DropdownMenu.Item
            className="menu-item"
            disabled={shells?.gitBash === null}
            onSelect={() => create('git_bash')}
          >
            <SquareTerminal /> Git Bash
          </DropdownMenu.Item>
          <DropdownMenu.Item
            className="menu-item"
            disabled={shells?.powershell === null}
            onSelect={() => create('powershell')}
          >
            <SquareTerminal /> PowerShell
          </DropdownMenu.Item>
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

/** The panel's top edge: drag (or use the arrow keys) to change its height. */
function ResizeHandle() {
  const height = useTerminal((s) => s.height);
  const setHeight = (next: number) =>
    useTerminal.getState().setHeight(Math.min(window.innerHeight - 220, next));
  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    const handle = event.currentTarget;
    const startY = event.clientY;
    const startHeight = useTerminal.getState().height;
    handle.setPointerCapture(event.pointerId);
    const move = (e: globalThis.PointerEvent) => setHeight(startHeight + startY - e.clientY);
    const up = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', up);
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', up);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.key === 'ArrowUp' ? 24 : event.key === 'ArrowDown' ? -24 : 0;
    if (!step) return;
    event.preventDefault();
    setHeight(height + step);
  };
  return (
    <div
      role="separator"
      aria-orientation="horizontal"
      aria-label="Terminal height"
      aria-valuenow={height}
      aria-valuemin={MIN_HEIGHT}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
      className="absolute inset-x-0 -top-1 z-10 h-2 cursor-row-resize outline-none focus-visible:bg-focus"
    />
  );
}

function seconds(ms: number): string {
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)} s`;
}

function statusOf(command: AgentCommand, running: boolean): string {
  if (running) return 'running…';
  switch (command.status) {
    case 'pending':
      return 'waiting';
    case 'denied':
      return 'not run';
    case 'cancelled':
      return 'stopped';
    default: {
      const parts: string[] = [];
      if (command.exitCode != null) {
        parts.push(command.exitCode === 0 ? 'done' : `exit ${command.exitCode}`);
      }
      if (command.durationMs != null) parts.push(seconds(command.durationMs));
      return parts.join(' · ');
    }
  }
}

/** The commands the agent ran in this session, with their output as it arrives. */
function AgentOutput() {
  const messages = useChat((s) => s.messages);
  const run = useChat((s) => s.run);
  const commands = useMemo(() => agentCommands(messages), [messages]);
  const box = useRef<HTMLDivElement>(null);
  const stick = useRef(true);

  useLayoutEffect(() => {
    const el = box.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [commands, run?.output]);

  return (
    <div
      ref={box}
      role="log"
      aria-label="Agent commands"
      onScroll={() => {
        const el = box.current;
        if (el) stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
      }}
      className="h-full overflow-y-auto px-4 pb-3 pt-2 font-mono text-[12.5px] leading-[1.55] select-text"
    >
      {commands.length === 0 ? (
        <p className="pt-1 font-sans text-[13px] text-text-3">
          Commands the agent runs in this session show here, with their output as it arrives.
        </p>
      ) : (
        commands.map((command) => {
          const running = command.status === 'pending' && run?.tool?.callId === command.callId;
          const output = (command.output ?? run?.output[command.callId] ?? '').replace(/\n$/, '');
          return (
            <div key={command.callId} className="mb-3 last:mb-0" data-call={command.callId}>
              <div className="flex items-baseline gap-2">
                <span className="text-accent-text select-none">$</span>
                <span className="min-w-0 flex-1 whitespace-pre-wrap break-words text-text">
                  {command.command}
                </span>
                <span className="shrink-0 font-sans text-[11.5px] text-text-3">
                  {statusOf(command, running)}
                </span>
              </div>
              {output && (
                <pre className="mt-0.5 whitespace-pre-wrap break-words text-text-2">{output}</pre>
              )}
            </div>
          );
        })
      )}
    </div>
  );
}
