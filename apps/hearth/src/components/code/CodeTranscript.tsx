import { ArrowDown, LoaderCircle, TriangleAlert, Undo2 } from 'lucide-react';
import { AlertDialog } from 'radix-ui';
import { memo, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { LiveRun } from '../../lib/runReducer';
import { answersByStep, sameAnswers } from '../../lib/toolCalls';
import type { Message, ToolCall } from '../../lib/types';
import { useChat } from '../../stores/chat';
import { AttachmentList } from '../Attachments';
import { SummaryDivider } from '../ContextMeter';
import { Markdown } from '../Markdown';
import { CopyButton, stats } from '../MessageView';
import { ThinkingBlock } from '../ThinkingBlock';
import { NoticeBanner, RunErrorBanner } from '../Transcript';
import { Button, IconButton } from '../ui/Button';
import { type CallState, ToolCard } from './ToolCard';
import { PermissionCard } from './PermissionCard';
import { QuestionCard } from './QuestionCard';

const NO_ANSWERS = new Map<string, Message>();

function stateOf(call: ToolCall, answer: Message | undefined, run: LiveRun | null): CallState {
  if (answer) return answer.toolStatus ?? 'ok';
  if (run?.permission?.callId === call.id || run?.question?.callId === call.id) return 'waiting';
  if (run?.tool?.callId === call.id) return 'running';
  return run ? 'queued' : 'not_run';
}

/**
 * Whether a step shows anything of the live run: it's being written, or has calls without an
 * answer yet. Other steps get no run, so they don't render again on every streamed token.
 */
function followsRun(message: Message, answers: Map<string, Message>, run: LiveRun | null) {
  if (!run) return false;
  return run.messageId === message.id || (message.toolCalls ?? []).some((c) => !answers.has(c.id));
}

/** A coding session: your requests, the agent's steps and its tool calls, and what it needs. */
export function CodeTranscript() {
  const messages = useChat((s) => s.messages);
  const run = useChat((s) => s.run);
  const notice = useChat((s) => s.notice);
  const steps = useMemo(() => answersByStep(messages), [messages]);
  const box = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const [away, setAway] = useState(false);

  useLayoutEffect(() => {
    const el = box.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [messages, run]);

  const onScroll = () => {
    const el = box.current;
    if (!el) return;
    const near = el.scrollHeight - el.scrollTop - el.clientHeight < 120;
    stick.current = near;
    setAway(!near);
  };

  const lastAssistant = messages.findLast((m) => m.role === 'assistant')?.id;
  // A task starts at the first user message of its turn (later ones were sent mid-task).
  const taskStarts = useMemo(() => {
    const seen = new Set<number>();
    const starts = new Set<string>();
    for (const m of messages) {
      if (m.role === 'user' && m.turn != null && !seen.has(m.turn)) {
        seen.add(m.turn);
        starts.add(m.id);
      }
    }
    return starts;
  }, [messages]);

  return (
    <div className="relative min-h-0 flex-1">
      <div
        ref={box}
        onScroll={onScroll}
        className="h-full overflow-y-auto"
        role="log"
        aria-label="Session"
      >
        <div className="transcript-list mx-auto max-w-[800px] space-y-5 px-6 pb-8 pt-8">
          {messages.map((message) =>
            message.role === 'user' ? (
              <UserTask
                key={message.id}
                message={message}
                undoable={taskStarts.has(message.id) && run === null}
              />
            ) : message.role === 'summary' ? (
              <SummaryDivider key={message.id} message={message} />
            ) : message.role === 'assistant' ? (
              <AgentStep
                key={message.id}
                message={message}
                answers={steps.get(message.id) ?? NO_ANSWERS}
                run={followsRun(message, steps.get(message.id) ?? NO_ANSWERS, run) ? run : null}
                last={message.id === lastAssistant}
              />
            ) : null,
          )}
          <LiveStatus run={run} messages={messages} />
          {run?.permission && (
            <PermissionCard key={run.permission.requestId} request={run.permission} />
          )}
          {run?.question && <QuestionCard key={run.question.questionId} question={run.question} />}
          {notice && <NoticeBanner notice={notice} />}
          <RunErrorBanner />
        </div>
      </div>
      {away && (
        <button
          type="button"
          aria-label="Jump to latest"
          onClick={() => {
            stick.current = true;
            box.current?.scrollTo({ top: box.current.scrollHeight, behavior: 'smooth' });
          }}
          className="absolute bottom-3 left-1/2 grid size-8 -translate-x-1/2 place-items-center rounded-full bg-surface text-text-2 shadow-pop ring-1 ring-line hover:text-text"
        >
          <ArrowDown className="size-4" />
        </button>
      )}
    </div>
  );
}

const UserTask = memo(function UserTask({
  message,
  undoable,
}: {
  message: Message;
  undoable: boolean;
}) {
  const [confirming, setConfirming] = useState(false);
  return (
    <div className="group flex flex-col items-end pt-1" data-role="user">
      <AttachmentList attachments={message.attachments ?? []} />
      {message.content && (
        <div className="max-w-[85%] whitespace-pre-wrap break-words rounded-[18px] bg-surface-2 px-4 py-2.5 text-[15px] leading-[1.6] select-text">
          {message.content}
        </div>
      )}
      <div className="mt-1 flex items-center opacity-0 transition-opacity duration-150 focus-within:opacity-100 group-hover:opacity-100">
        <CopyButton text={message.content} />
        {undoable && (
          <IconButton label="Undo file changes from here" onClick={() => setConfirming(true)}>
            <Undo2 />
          </IconButton>
        )}
      </div>
      <AlertDialog.Root open={confirming} onOpenChange={setConfirming}>
        <AlertDialog.Portal>
          <AlertDialog.Overlay className="dialog-overlay" />
          <AlertDialog.Content className="dialog w-[420px]">
            <AlertDialog.Title className="text-[15px] font-semibold">
              Undo the file changes from this task on?
            </AlertDialog.Title>
            <AlertDialog.Description className="mt-1.5 text-[13.5px] leading-relaxed text-text-2">
              Files the agent changed in this task and the ones after it go back to how they were
              before it. The conversation stays as it is, and changes made by commands aren't
              undone.
            </AlertDialog.Description>
            <div className="mt-5 flex justify-end gap-2">
              <AlertDialog.Cancel asChild>
                <Button variant="ghost">Cancel</Button>
              </AlertDialog.Cancel>
              <AlertDialog.Action asChild>
                <Button
                  variant="primary"
                  onClick={() => void useChat.getState().undo(message.turn ?? 1)}
                >
                  Undo changes
                </Button>
              </AlertDialog.Action>
            </div>
          </AlertDialog.Content>
        </AlertDialog.Portal>
      </AlertDialog.Root>
    </div>
  );
});

type AgentStepProps = {
  message: Message;
  answers: Map<string, Message>;
  /** The live run, for the steps that show it (see followsRun). */
  run: LiveRun | null;
  last: boolean;
};

/** One step of the agent: thinking, text and tool calls. Memoized like chat messages. */
const AgentStep = memo(
  AgentStepView,
  (prev, next) =>
    prev.message === next.message &&
    prev.run === next.run &&
    prev.last === next.last &&
    sameAnswers(prev.answers, next.answers),
);

function AgentStepView({ message, answers, run, last }: AgentStepProps) {
  const streaming = run?.messageId === message.id && message.status === 'streaming';
  const calls = message.toolCalls ?? [];
  const reasoning = message.reasoning ?? '';
  const info = stats(message);
  const final = !streaming && !calls.length && message.status === 'complete' && !!message.content;
  if (!message.content && !reasoning && !calls.length && message.status !== 'error') {
    return message.status === 'interrupted' ? (
      <p className="text-[12.5px] text-text-3" data-role="assistant">
        Stopped
      </p>
    ) : null;
  }
  // Steps that are only tool calls sit close to the step before.
  const toolsOnly = !message.content && !reasoning && calls.length > 0;
  return (
    <div
      className={toolsOnly ? 'group -mt-3.5' : 'group'}
      data-role="assistant"
      data-status={message.status}
    >
      {reasoning && (
        <ThinkingBlock
          text={reasoning}
          ms={message.reasoningMs}
          live={streaming && !message.content && !calls.length}
        />
      )}
      {message.content && <Markdown text={message.content} streaming={streaming} />}
      {calls.length > 0 && (
        <div className={message.content ? 'mt-2.5 space-y-0.5' : 'space-y-0.5'}>
          {calls.map((call) => {
            const answer = answers.get(call.id);
            return (
              <ToolCard
                key={call.id}
                call={call}
                answer={answer}
                state={stateOf(call, answer, run)}
                liveTitle={run?.tool?.callId === call.id ? run.tool.title : undefined}
                liveOutput={run?.output[call.id]}
              />
            );
          })}
        </div>
      )}
      {message.status === 'error' && (
        <div className="mt-2 flex items-start gap-2 rounded-card bg-danger-soft px-3.5 py-2.5 text-[13.5px] text-danger">
          <TriangleAlert className="mt-0.5 size-4 shrink-0" />
          <span className="min-w-0 flex-1 select-text">{message.error ?? 'This step failed.'}</span>
        </div>
      )}
      {message.status === 'interrupted' && (
        <p className="mt-2 text-[12.5px] text-text-3">Stopped</p>
      )}
      {final && (
        <div
          className={
            last
              ? 'mt-1.5 flex items-center'
              : 'mt-1.5 flex items-center opacity-0 transition-opacity duration-150 focus-within:opacity-100 group-hover:opacity-100'
          }
        >
          <CopyButton text={message.content} />
          {info && <span className="ml-2 truncate text-[12px] text-text-3">{info}</span>}
        </div>
      )}
    </div>
  );
}

/** What the agent is doing when nothing else on screen shows it. */
function LiveStatus({ run, messages }: { run: LiveRun | null; messages: Message[] }) {
  if (!run || run.permission || run.question || run.tool) return null;
  let text = 'Thinking…';
  if (run.compacting) {
    text = 'Summarizing the earlier conversation to make room…';
  } else if (run.loading) {
    text = `Loading ${run.loading}… the first reply can take a minute.`;
  } else if (run.writing) {
    const size =
      run.writing.chars >= 1000 ? `${(run.writing.chars / 1000).toFixed(1)}k` : run.writing.chars;
    text = `Writing ${run.writing.name ?? 'a tool call'} · ${size} characters`;
  } else {
    const live = run.messageId ? messages.find((m) => m.id === run.messageId) : undefined;
    if (live && (live.content || live.reasoning)) return null;
    if (run.progress && run.progress.total > 0) {
      const percent = Math.min(
        100,
        Math.round((run.progress.processed / run.progress.total) * 100),
      );
      text = `Reading the session… ${percent}%`;
    }
  }
  return (
    <div className="flex items-center gap-2 px-2 text-[13px] text-text-2" aria-live="polite">
      <LoaderCircle className="size-3.5 animate-spin text-accent" />
      <span className="thinking-shimmer">{text}</span>
    </div>
  );
}
