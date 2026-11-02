import {
  Ban,
  ChevronRight,
  Circle,
  CircleDot,
  CircleSlash,
  FilePen,
  FilePlus,
  FileSearch,
  FileText,
  FolderTree,
  ListTodo,
  LoaderCircle,
  MessageCircleQuestionMark,
  OctagonX,
  ScrollText,
  Search,
  SquareTerminal,
  TriangleAlert,
  Wrench,
} from 'lucide-react';
import { type ReactNode, useLayoutEffect, useRef, useState } from 'react';
import { cn } from '../../lib/cn';
import { diffStats } from '../../lib/diff';
import { callTitle, canonicalTool, parseArgs } from '../../lib/toolCalls';
import type { Message, ToolCall, ToolStatus } from '../../lib/types';
import { DiffView } from './DiffView';
import { TodoList } from './TodoPanel';

/** Where a call is: done (its answer's status), or still on its way. */
export type CallState = ToolStatus | 'running' | 'waiting' | 'queued' | 'not_run';

const ICONS: Record<string, typeof FileText> = {
  read_file: FileText,
  list_dir: FolderTree,
  glob: FileSearch,
  grep: Search,
  write_file: FilePlus,
  edit_file: FilePen,
  todo_write: ListTodo,
  ask_user: MessageCircleQuestionMark,
  run_command: SquareTerminal,
  command_output: ScrollText,
  kill_process: OctagonX,
};

function StateIcon({ state, tool }: { state: CallState; tool: string }) {
  const Icon = ICONS[tool] ?? Wrench;
  const icon = (node: ReactNode, label: string) => (
    <span
      className="grid size-4 shrink-0 place-items-center [&_svg]:size-[15px]"
      aria-label={label}
    >
      {node}
    </span>
  );
  switch (state) {
    case 'running':
      return icon(<LoaderCircle className="animate-spin text-accent" />, 'Running');
    case 'waiting':
      return icon(<CircleDot className="animate-pulse text-accent" />, 'Waiting for you');
    case 'queued':
      return icon(<Circle className="text-text-3" />, 'Queued');
    case 'error':
      return icon(<TriangleAlert className="text-danger" />, 'Failed');
    case 'denied':
      return icon(<Ban className="text-text-3" />, 'Declined');
    case 'cancelled':
    case 'not_run':
      return icon(<CircleSlash className="text-text-3" />, 'Not run');
    default:
      return icon(<Icon className="stroke-[1.75] text-text-2" />, 'Done');
  }
}

function summaryOf(state: CallState, answer: Message | undefined): string {
  if (answer?.toolDisplay?.summary) return answer.toolDisplay.summary;
  switch (state) {
    case 'running':
      return 'Running…';
    case 'waiting':
      return 'Waiting for you';
    case 'queued':
      return 'Up next';
    case 'not_run':
      return 'Not run';
    default:
      return '';
  }
}

/** A command and what it printed, following the end while it runs. */
function CommandView({
  command,
  output,
  running,
}: {
  command: string;
  output: string | undefined;
  running: boolean;
}) {
  const box = useRef<HTMLPreElement>(null);
  useLayoutEffect(() => {
    if (running && box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [output, running]);
  return (
    <pre
      ref={box}
      aria-label="Command output"
      className="max-h-[260px] overflow-auto whitespace-pre-wrap break-words rounded-[10px] bg-surface-2 px-3 py-2 font-mono text-[12px] leading-[1.55] text-text-2 select-text"
    >
      <span className="text-text">
        <span className="text-text-3 select-none">$ </span>
        {command}
      </span>
      {output ? `\n${output.replace(/\n$/, '')}` : running ? '\n' : '\n(no output)'}
      {running && (
        <span className="ml-0.5 inline-block h-3 w-1.5 animate-pulse bg-text-3 align-middle" />
      )}
    </pre>
  );
}

/** One tool call as a compact row; opens to show its diff, command output or result. */
export function ToolCard({
  call,
  answer,
  state,
  liveTitle,
  liveOutput,
}: {
  call: ToolCall;
  answer: Message | undefined;
  state: CallState;
  liveTitle?: string;
  /** A running command's output so far. */
  liveOutput?: string;
}) {
  const display = answer?.toolDisplay ?? null;
  const tool = answer?.toolName ?? canonicalTool(call.function.name);
  const title = display?.title ?? liveTitle ?? callTitle(call);
  const diff = display?.diff;
  const command =
    tool === 'run_command'
      ? (display?.command ?? (parseArgs(call).command as string | undefined) ?? '')
      : null;
  // Changes show their diff, and commands their output while running or after failing, until
  // the user folds them away.
  const [toggled, setToggled] = useState<boolean | null>(null);
  const open =
    toggled ??
    ((!!diff && state === 'ok') ||
      (command !== null && (state === 'running' || state === 'error')));
  const stats = diff ? diffStats(diff) : null;
  const summary = summaryOf(state, answer);
  const output = answer?.content ?? '';
  const showOutput = command === null && !!output && !(diff && state === 'ok') && !display?.todos;
  const expandable = !!(
    diff ||
    display?.todos?.length ||
    display?.note ||
    showOutput ||
    (command !== null && (answer || state === 'running'))
  );

  return (
    <div data-tool={tool} data-status={state} className="tool-card">
      <button
        type="button"
        aria-expanded={expandable ? open : undefined}
        disabled={!expandable}
        onClick={() => setToggled(!open)}
        className="group flex h-8 w-full items-center gap-2.5 rounded-control px-2 text-left text-[13px] transition-colors duration-150 enabled:hover:bg-surface-2"
      >
        <StateIcon state={state} tool={tool} />
        <span className="min-w-0 shrink truncate font-medium text-text" title={title}>
          {title}
        </span>
        {stats && (stats.added > 0 || stats.removed > 0) && (
          <span className="shrink-0 font-mono text-[11.5px] tabular-nums">
            <span className="text-[var(--diff-add-mark)]">+{stats.added}</span>{' '}
            <span className="text-[var(--diff-del-mark)]">−{stats.removed}</span>
          </span>
        )}
        <span
          className={cn(
            'min-w-0 flex-1 truncate text-[12.5px]',
            state === 'error' ? 'text-danger' : 'text-text-3',
          )}
        >
          {summary && summary !== title ? summary : ''}
        </span>
        {expandable && (
          <ChevronRight
            className={cn(
              'size-3.5 shrink-0 text-text-3 transition-transform duration-200',
              open && 'rotate-90',
            )}
          />
        )}
      </button>
      {open && expandable && (
        <div className="mb-2 ml-[34px] mt-1 space-y-2">
          {diff && <DiffView diff={diff} />}
          {command !== null && (
            <CommandView
              command={command}
              output={display?.output ?? liveOutput}
              running={state === 'running'}
            />
          )}
          {display?.todos && <TodoList todos={display.todos} />}
          {display?.note && (
            <p className="text-[12.5px] text-text-2">
              <span className="text-text-3">Your note:</span> {display.note}
            </p>
          )}
          {showOutput && (
            <pre className="max-h-[260px] overflow-auto whitespace-pre-wrap break-words rounded-[10px] bg-surface-2 px-3 py-2 font-mono text-[12px] leading-[1.55] text-text-2 select-text">
              {output}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}
