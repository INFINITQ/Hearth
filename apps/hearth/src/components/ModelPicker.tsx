import { Check, ChevronDown, Settings2 } from 'lucide-react';
import { DropdownMenu } from 'radix-ui';
import { useState } from 'react';
import { backend, errorMessage } from '../lib/backend';
import { cn } from '../lib/cn';
import type { Connection, LocalModel, ModelRef, ModelStatus } from '../lib/types';
import { useApp } from '../stores/app';
import { useChat } from '../stores/chat';
import { useModels } from '../stores/models';

type Models = Record<string, string[] | { error: string }>;

// Stable empty lists: a selector that returns a new [] each time re-renders for ever.
const NO_MODELS: LocalModel[] = [];
const NO_STATUS: ModelStatus[] = [];

function same(a: ModelRef | null, b: ModelRef) {
  if (!a || a.kind !== b.kind) return false;
  if (a.kind === 'local' && b.kind === 'local') return a.modelId === b.modelId;
  if (a.kind === 'connection' && b.kind === 'connection')
    return a.connectionId === b.connectionId && a.model === b.model;
  return false;
}

/** The model for the open conversation (or the default for new ones), in the title bar. */
export function ModelPicker() {
  const mode = useApp((s) => s.mode);
  const defaults = useApp((s) => s.settings.models);
  const session = useChat((s) => s.session);
  const local = useModels((s) => s.overview?.models ?? NO_MODELS);
  const status = useModels((s) => s.overview?.status ?? NO_STATUS);
  const own = session && session.mode === mode ? session.modelRef : null;
  const current = own ?? defaults[mode];
  const [connections, setConnections] = useState<Connection[] | null>(null);
  const [models, setModels] = useState<Models>({});

  const label =
    current?.kind === 'local'
      ? (local.find((m) => m.id === current.modelId)?.name ?? 'A model on this PC')
      : current?.model;
  const ready = (id: string) => status.some((s) => s.modelId === id && s.state === 'ready');

  const load = async () => {
    void useModels.getState().load();
    const list = await backend().connectionsList();
    setConnections(list);
    for (const connection of list) {
      backend()
        .connectionModels(connection.id)
        .then((found) => setModels((prev) => ({ ...prev, [connection.id]: found })))
        .catch((err: unknown) =>
          setModels((prev) => ({ ...prev, [connection.id]: { error: errorMessage(err) } })),
        );
    }
  };

  const choose = (model: ModelRef) => {
    if (session && session.mode === mode) void useChat.getState().setModel(model);
    else void useApp.getState().updateSettings({ models: { [mode]: model } });
  };

  return (
    <DropdownMenu.Root onOpenChange={(open) => open && void load()}>
      <DropdownMenu.Trigger asChild>
        <button
          type="button"
          aria-label="Model"
          className="mr-1 flex h-7 max-w-[280px] items-center gap-2 rounded-full px-3 text-[12.5px] font-medium text-text-2 transition-colors duration-150 hover:bg-surface-3/70 hover:text-text data-[state=open]:bg-surface-3"
        >
          <span
            className={cn('size-1.5 shrink-0 rounded-full', current ? 'bg-success' : 'bg-text-3')}
          />
          <span className="truncate">{label ?? 'Choose a model'}</span>
          <ChevronDown className="size-3.5 shrink-0" />
        </button>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content
          align="end"
          sideOffset={6}
          className="menu max-h-[70vh] min-w-[280px] max-w-[380px] overflow-y-auto"
        >
          {local.length > 0 && (
            <DropdownMenu.Group>
              <DropdownMenu.Label className="menu-label">On this PC</DropdownMenu.Label>
              {local.map((model) => {
                const ref: ModelRef = { kind: 'local', modelId: model.id };
                return (
                  <DropdownMenu.Item
                    key={model.id}
                    className="menu-item"
                    onSelect={() => choose(ref)}
                  >
                    <Check className={cn(!same(current, ref) && 'invisible')} />
                    <span className="min-w-0 flex-1 truncate">{model.name}</span>
                    {model.quant && (
                      <span className="shrink-0 font-mono text-[11px] text-text-3">
                        {model.quant}
                      </span>
                    )}
                    {ready(model.id) && (
                      <span
                        className="size-1.5 shrink-0 rounded-full bg-success"
                        title="Loaded"
                        aria-label="Loaded"
                      />
                    )}
                  </DropdownMenu.Item>
                );
              })}
            </DropdownMenu.Group>
          )}
          {connections === null && <p className="menu-note">Loading…</p>}
          {connections?.length === 0 && local.length === 0 && (
            <p className="menu-note">No models yet.</p>
          )}
          {connections?.map((connection) => {
            const found = models[connection.id];
            return (
              <DropdownMenu.Group key={connection.id}>
                <DropdownMenu.Label className="menu-label">{connection.name}</DropdownMenu.Label>
                {found === undefined && <p className="menu-note">Looking for models…</p>}
                {found && 'error' in found && (
                  <p className="menu-note text-danger">Not reachable</p>
                )}
                {Array.isArray(found) &&
                  found.map((model) => {
                    const ref: ModelRef = {
                      kind: 'connection',
                      connectionId: connection.id,
                      model,
                    };
                    return (
                      <DropdownMenu.Item
                        key={model}
                        className="menu-item"
                        onSelect={() => choose(ref)}
                      >
                        <Check className={cn(!same(current, ref) && 'invisible')} />
                        <span className="truncate">{model}</span>
                      </DropdownMenu.Item>
                    );
                  })}
              </DropdownMenu.Group>
            );
          })}
          <DropdownMenu.Separator className="menu-sep" />
          <DropdownMenu.Item className="menu-item" onSelect={() => useApp.getState().go('models')}>
            <Settings2 /> Manage models…
          </DropdownMenu.Item>
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}
