import { Boxes, Download, FolderPlus, Wrench } from 'lucide-react';
import type { ReactNode } from 'react';

/** Models page. Until llama.cpp and a model are set up it explains the three steps. */
export function ModelsScreen() {
  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto max-w-[760px] px-8 pb-16 pt-6">
        <h1 className="text-[22px] font-semibold tracking-[-0.018em]">Models</h1>
        <p className="mt-1 text-[14px] text-text-2">
          Hearth runs models on your PC with llama.cpp. Nothing leaves your machine.
        </p>

        <div className="mt-8 rounded-card bg-surface p-6 shadow-float ring-1 ring-line">
          <div className="flex items-center gap-3">
            <div className="grid size-10 place-items-center rounded-control bg-accent-soft text-accent">
              <Boxes className="size-5 stroke-[1.75]" />
            </div>
            <div>
              <h2 className="text-[15px] font-semibold">No models yet</h2>
              <p className="text-[13px] text-text-2">
                Three steps, and the guide in docs/models.md walks you through them.
              </p>
            </div>
          </div>
          <ol className="mt-6 space-y-4">
            <Step icon={<Wrench />} n={1} title="Install llama.cpp">
              The Windows CUDA 13 build from github.com/ggml-org/llama.cpp/releases.
            </Step>
            <Step icon={<Download />} n={2} title="Download a GGUF model">
              For your RTX 5070: Qwen3.6-35B-A3B for coding, Gemma 4 26B-A4B for chat.
            </Step>
            <Step icon={<FolderPlus />} n={3} title="Add its folder here">
              Hearth finds the models, picks good settings, and loads them when you need them.
            </Step>
          </ol>
        </div>
      </div>
    </div>
  );
}

function Step({
  icon,
  n,
  title,
  children,
}: {
  icon: ReactNode;
  n: number;
  title: string;
  children: ReactNode;
}) {
  return (
    <li className="flex gap-3">
      <span className="mt-0.5 grid size-6 shrink-0 place-items-center rounded-full bg-surface-3 text-[12px] font-semibold text-text-2">
        {n}
      </span>
      <div className="min-w-0">
        <p className="flex items-center gap-1.5 text-[14px] font-medium [&_svg]:size-3.5 [&_svg]:text-text-3">
          {icon}
          {title}
        </p>
        <p className="mt-0.5 text-[13px] leading-relaxed text-text-2">{children}</p>
      </div>
    </li>
  );
}
