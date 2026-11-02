import { FolderOpen, FolderPlus, SquareTerminal } from 'lucide-react';
import { useMemo } from 'react';
import { Composer } from '../components/Composer';
import { RunErrorBanner } from '../components/Transcript';
import { CodeTranscript } from '../components/code/CodeTranscript';
import { ModeSwitch, cycleMode } from '../components/code/ModeSwitch';
import { TodoPanel } from '../components/code/TodoPanel';
import { TrustDialog } from '../components/code/TrustDialog';
import { Button } from '../components/ui/Button';
import { latestTodos } from '../lib/toolCalls';
import type { Project } from '../lib/types';
import { useApp } from '../stores/app';
import { useChat } from '../stores/chat';

function send(text: string) {
  void useChat.getState().send(text, 'code');
}

function stop() {
  useChat.getState().stop();
}

/** Code mode: pick a project, then work with the agent in sessions. */
export function CodeScreen() {
  const project = useChat((s) => s.project);
  const session = useChat((s) => s.session);
  const messages = useChat((s) => s.messages);
  const run = useChat((s) => s.run);
  const started = session?.mode === 'code' && (messages.length > 0 || run !== null);
  return (
    <>
      {!project ? (
        <OpenProject />
      ) : started ? (
        <Session project={project} />
      ) : (
        <NewSession project={project} />
      )}
      <TrustDialog />
    </>
  );
}

function OpenProject() {
  const projects = useChat((s) => s.projects);
  return (
    <div className="flex min-h-0 flex-1 items-center justify-center overflow-y-auto px-6 pb-[10vh]">
      <div className="w-full max-w-[460px]">
        <div className="text-center">
          <div className="mx-auto mb-5 grid size-12 place-items-center rounded-card bg-accent-soft text-accent">
            <SquareTerminal className="size-6 stroke-[1.75]" />
          </div>
          <h1 className="text-[22px] font-semibold tracking-[-0.018em]">
            Open a project to start coding
          </h1>
          <p className="mt-2 text-[14px] leading-relaxed text-text-2">
            Hearth's agent works in the folder you choose: it reads and edits files, and asks before
            anything risky.
          </p>
          <Button
            variant="primary"
            className="mt-6"
            onClick={() => void useChat.getState().pickProject()}
          >
            <FolderPlus />
            Open folder…
          </Button>
        </div>
        {projects.length > 0 && (
          <div className="mt-9">
            <p className="px-2.5 pb-1 text-[11.5px] font-medium text-text-3">Recent</p>
            {projects.slice(0, 6).map((item) => (
              <button
                key={item.id}
                type="button"
                disabled={!item.exists}
                onClick={() => void useChat.getState().openProject(item)}
                className="flex w-full items-center gap-3 rounded-control px-2.5 py-2 text-left transition-colors duration-150 enabled:hover:bg-surface-3/60 disabled:opacity-50"
              >
                <FolderOpen className="size-4 shrink-0 stroke-[1.75] text-text-3" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[13.5px] font-medium text-text">
                    {item.name}
                  </span>
                  <span className="block truncate text-[12px] text-text-3">{item.path}</span>
                </span>
                {!item.exists && <span className="text-[11.5px] text-text-3">Missing</span>}
              </button>
            ))}
          </div>
        )}
        <div className="mt-4">
          <RunErrorBanner />
        </div>
      </div>
    </div>
  );
}

function NewSession({ project }: { project: Project }) {
  const model = useApp((s) => s.settings.models.code);
  const run = useChat((s) => s.run);
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center px-6 pb-[14vh]">
      <div className="w-full max-w-[720px]">
        <h1 className="text-center text-[27px] font-semibold tracking-[-0.022em] text-text">
          What should we work on?
        </h1>
        <p className="mb-7 mt-1.5 text-center text-[13.5px] text-text-3" title={project.path}>
          in <span className="font-medium text-text-2">{project.name}</span>
        </p>
        <Composer
          placeholder="Describe a task, or ask about the code…"
          running={run !== null}
          onSend={send}
          onStop={stop}
          tools={<ModeSwitch />}
          onShiftTab={cycleMode}
          autoFocus
        />
        <div className="mt-4">
          <RunErrorBanner />
        </div>
        {!model && (
          <p className="mt-3.5 text-center text-[12.5px] text-text-3">
            No coding model is set up yet.{' '}
            <button
              type="button"
              className="font-medium text-accent-text hover:underline"
              onClick={() => useApp.getState().go('settings')}
            >
              Choose one
            </button>
          </p>
        )}
      </div>
    </div>
  );
}

function Session({ project }: { project: Project }) {
  const run = useChat((s) => s.run);
  const messages = useChat((s) => s.messages);
  const todos = useMemo(() => latestTodos(messages), [messages]);
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <CodeTranscript />
      <div className="mx-auto w-full max-w-[800px] px-6 pb-3">
        {todos.length > 0 && <TodoPanel todos={todos} />}
        <Composer
          placeholder={run ? 'Working… (Esc stops)' : 'Describe the next step…'}
          running={run !== null}
          onSend={send}
          onStop={stop}
          tools={<ModeSwitch />}
          onShiftTab={cycleMode}
          autoFocus
        />
        <p className="mt-2 truncate text-center text-[11.5px] text-text-3" title={project.path}>
          Working in {project.path}. Check changes before you rely on them.
        </p>
      </div>
    </div>
  );
}
