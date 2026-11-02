// First run: the three steps to a working model, each with where to do it.

import { Check } from 'lucide-react';
import { cn } from '../lib/cn';
import { type Screen, useApp } from '../stores/app';
import { useModels } from '../stores/models';
import { Button } from './ui/Button';

type Step = { done: boolean; title: string; text: string; action: string; screen: Screen };

/** Shown where a conversation would start when no model is chosen for it yet. */
export function SetupCard({ role }: { role: 'chat' | 'code' }) {
  const overview = useModels((s) => s.overview);
  const server = overview?.server ?? null;
  const count = overview?.models.length ?? 0;
  const steps: Step[] = [
    {
      done: server !== null,
      title: 'Install llama.cpp',
      text: server
        ? `Found ${server.version ? `llama.cpp ${server.version}` : 'llama-server'}.`
        : 'Its CUDA build runs models on your graphics card. The guide shows how.',
      action: 'Open the guide',
      screen: 'guide',
    },
    {
      done: count > 0,
      title: 'Get a model',
      text:
        count > 0
          ? `${count} model file${count === 1 ? '' : 's'} found.`
          : 'Download one the guide suggests, then add its folder on the Models page.',
      action: 'Add models',
      screen: 'models',
    },
    {
      done: false,
      title: role === 'chat' ? 'Choose the chat model' : 'Choose the coding model',
      text: 'On the Models page. A server you already run (LM Studio, Ollama) works too: add it in Settings.',
      action: 'Choose',
      screen: 'models',
    },
  ];
  return (
    <section
      aria-label="Get started"
      className="mt-6 rounded-card bg-surface p-4 text-left shadow-float ring-1 ring-line"
    >
      <p className="text-[14px] font-semibold">Get started</p>
      <p className="mt-0.5 text-[12.5px] text-text-2">
        Hearth runs models on this PC. These steps are needed once.
      </p>
      <ol className="mt-3 space-y-2.5">
        {steps.map((step, index) => (
          <li key={step.title} className="flex items-center gap-3" data-done={step.done}>
            <span
              className={cn(
                'grid size-6 shrink-0 place-items-center rounded-full text-[12px] font-semibold',
                step.done ? 'bg-success/15 text-success' : 'bg-surface-3 text-text-2',
              )}
            >
              {step.done ? <Check className="size-3.5" /> : index + 1}
            </span>
            <div className="min-w-0 flex-1">
              <p className="text-[13.5px] font-medium">{step.title}</p>
              <p className="text-[12.5px] text-text-2">{step.text}</p>
            </div>
            <Button
              size="sm"
              variant={step.done ? 'ghost' : 'secondary'}
              onClick={() => useApp.getState().go(step.screen)}
            >
              {step.action}
            </Button>
          </li>
        ))}
      </ol>
    </section>
  );
}
