import { FolderOpen, SquareTerminal } from 'lucide-react';
import { Button } from '../components/ui/Button';

/** Code mode before a project is open. */
export function CodeScreen() {
  return (
    <div className="flex min-h-0 flex-1 items-center justify-center px-6 pb-[12vh]">
      <div className="max-w-[460px] text-center">
        <div className="mx-auto mb-5 grid size-12 place-items-center rounded-card bg-accent-soft text-accent">
          <SquareTerminal className="size-6 stroke-[1.75]" />
        </div>
        <h1 className="text-[22px] font-semibold tracking-[-0.018em]">
          Open a project to start coding
        </h1>
        <p className="mt-2 text-[14px] leading-relaxed text-text-2">
          Hearth's agent works in the folder you choose: it reads and edits files, runs commands,
          and asks before anything risky.
        </p>
        <Button variant="primary" className="mt-6" disabled>
          <FolderOpen />
          Open folder…
        </Button>
      </div>
    </div>
  );
}
