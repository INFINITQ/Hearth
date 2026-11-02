import { Check, ChevronDown, Settings2 } from 'lucide-react';
import { DropdownMenu } from 'radix-ui';
import { useState } from 'react';
import { backend, errorMessage } from '../lib/backend';
import { cn } from '../lib/cn';
import type { Connection, ModelRef } from '../lib/types';
import { useApp } from '../stores/app';
import { useChat } from '../stores/chat';

type Models = Record<string, string[] | { error: string }>;

function same(a: ModelRef | null, b: ModelRef) {
  return !!a && a.connectionId === b.connectionId && a.model === b.model;
}

/** The model for the open chat (or the default for new ones), in the title bar. */
export function ModelPicker() {
  const mode = useApp((s) => s.mode);
  const defaults = useApp((s) => s.settings.models);
  const session = useChat((s) => s.session);
  const own = session && session.mode === mode ? session.modelRef : null;
  const current = own ?? defaults[mode];
  const [connections, setConnections] = useState<Connection[] | null>(null);
  const [models, setModels] = useState<Models>({});

  const load = async () => {
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
          <span className="truncate">{current?.model ?? 'Choose a model'}</span>
          <ChevronDown className="size-3.5 shrink-0" />
        </button>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content
          align="end"
          sideOffset={6}
          className="menu min-w-[260px] max-w-[360px]"
        >
          {connections === null && <p className="menu-note">Loading…</p>}
          {connections?.length === 0 && <p className="menu-note">No connections yet.</p>}
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
          <DropdownMenu.Item
            className="menu-item"
            onSelect={() => useApp.getState().go('settings')}
          >
            <Settings2 /> Manage models…
          </DropdownMenu.Item>
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}
