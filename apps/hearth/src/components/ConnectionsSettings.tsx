import { Plug, Trash2 } from 'lucide-react';
import { useEffect, useState } from 'react';
import { backend, errorMessage } from '../lib/backend';
import type { Connection, ConnectionInput, Mode, ModelRef } from '../lib/types';
import { useApp } from '../stores/app';
import { Button, IconButton } from './ui/Button';

type Choice = { label: string; ref: ModelRef };

/** Servers Hearth talks to, plus the default model for chats and for coding. */
export function ConnectionsSettings() {
  const [connections, setConnections] = useState<Connection[]>([]);
  const [choices, setChoices] = useState<Choice[]>([]);
  const [editing, setEditing] = useState<ConnectionInput | null>(null);

  const reload = async () => {
    const list = await backend().connectionsList();
    setConnections(list);
    const local = await backend()
      .runtimeOverview(false)
      .then((overview) => overview.models)
      .catch(() => []);
    const found: Choice[] = local.map((model) => ({
      label: `${model.name}${model.quant ? ` · ${model.quant}` : ''} · this PC`,
      ref: { kind: 'local', modelId: model.id },
    }));
    await Promise.all(
      list.map(async (connection) => {
        const models = await backend()
          .connectionModels(connection.id)
          .catch(() => [] as string[]);
        for (const model of models) {
          found.push({
            label: `${model} · ${connection.name}`,
            ref: { kind: 'connection', connectionId: connection.id, model },
          });
        }
      }),
    );
    setChoices(found);
  };

  useEffect(() => {
    void reload();
  }, []);

  return (
    <>
      <div className="divide-y divide-line">
        <DefaultModel mode="chat" label="Chat model" choices={choices} />
        <DefaultModel mode="code" label="Coding model" choices={choices} />
      </div>
      <div className="border-t border-line px-5 py-3.5">
        <div className="flex items-center justify-between">
          <div>
            <p className="text-[14px] font-medium">Connections</p>
            <p className="mt-0.5 text-[12.5px] text-text-2">
              A llama-server you started yourself, LM Studio, Ollama, or any OpenAI-compatible
              server.
            </p>
          </div>
          {!editing && (
            <Button
              size="sm"
              onClick={() =>
                setEditing({ name: '', baseUrl: 'http://127.0.0.1:8080', llamaExtensions: true })
              }
            >
              <Plug /> Add
            </Button>
          )}
        </div>
        {connections.length > 0 && (
          <ul className="mt-3 space-y-1.5">
            {connections.map((connection) => (
              <li
                key={connection.id}
                className="flex items-center gap-3 rounded-control bg-surface-2 px-3 py-2"
              >
                <div className="min-w-0 flex-1">
                  <p className="truncate text-[13px] font-medium">{connection.name}</p>
                  <p className="truncate font-mono text-[11.5px] text-text-3">
                    {connection.baseUrl}
                  </p>
                </div>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() =>
                    setEditing({
                      id: connection.id,
                      name: connection.name,
                      baseUrl: connection.baseUrl,
                      llamaExtensions: connection.llamaExtensions,
                    })
                  }
                >
                  Edit
                </Button>
                <IconButton
                  label={`Remove ${connection.name}`}
                  onClick={() => void backend().connectionDelete(connection.id).then(reload)}
                >
                  <Trash2 />
                </IconButton>
              </li>
            ))}
          </ul>
        )}
        {editing && (
          <ConnectionForm
            initial={editing}
            onDone={() => {
              setEditing(null);
              void reload();
            }}
          />
        )}
      </div>
    </>
  );
}

/** One string per model reference, for `<select>` values. */
function refKey(ref: ModelRef): string {
  return ref.kind === 'local'
    ? `local\u0000${ref.modelId}`
    : `connection\u0000${ref.connectionId}\u0000${ref.model}`;
}

function DefaultModel({ mode, label, choices }: { mode: Mode; label: string; choices: Choice[] }) {
  const current = useApp((s) => s.settings.models[mode]);
  const value = current ? refKey(current) : '';
  const known = choices.some((c) => refKey(c.ref) === value);
  return (
    <div className="flex items-center gap-6 px-5 py-3.5">
      <p className="min-w-0 flex-1 text-[14px] font-medium">{label}</p>
      <select
        aria-label={label}
        value={known ? value : ''}
        onChange={(e) => {
          const choice = choices.find((c) => refKey(c.ref) === e.target.value);
          void useApp.getState().updateSettings({ models: { [mode]: choice?.ref ?? null } });
        }}
        className="h-8 max-w-[300px] rounded-control bg-surface-2 px-2.5 text-[13px] outline-none ring-1 ring-line focus:ring-accent"
      >
        <option value="">
          {current && !known
            ? current.kind === 'local'
              ? 'A model on this PC'
              : current.model
            : 'Choose…'}
        </option>
        {choices.map((choice) => (
          <option key={refKey(choice.ref)} value={refKey(choice.ref)}>
            {choice.label}
          </option>
        ))}
      </select>
    </div>
  );
}

function ConnectionForm({ initial, onDone }: { initial: ConnectionInput; onDone(): void }) {
  const [form, setForm] = useState<ConnectionInput>(initial);
  const [apiKey, setApiKey] = useState('');
  const [status, setStatus] = useState<{ ok: boolean; text: string } | null>(null);
  const field =
    'h-8 w-full rounded-control bg-surface px-2.5 text-[13px] outline-none ring-1 ring-line focus:ring-accent select-text';

  const test = async () => {
    setStatus({ ok: true, text: 'Checking…' });
    try {
      const models = await backend().connectionTest(form.baseUrl, apiKey || null);
      setStatus({
        ok: true,
        text: models.length ? `Found: ${models.join(', ')}` : 'Reachable, but it lists no models.',
      });
    } catch (err) {
      setStatus({ ok: false, text: errorMessage(err) });
    }
  };

  const save = async () => {
    try {
      await backend().connectionSave({ ...form, apiKey: apiKey ? apiKey : undefined });
      onDone();
    } catch (err) {
      setStatus({ ok: false, text: errorMessage(err) });
    }
  };

  return (
    <div className="mt-3 space-y-2.5 rounded-control bg-surface-2 p-3.5">
      <input
        aria-label="Name"
        placeholder="Name (e.g. LM Studio)"
        className={field}
        value={form.name}
        onChange={(e) => setForm({ ...form, name: e.target.value })}
      />
      <input
        aria-label="Address"
        placeholder="http://127.0.0.1:8080"
        className={`${field} font-mono`}
        value={form.baseUrl}
        onChange={(e) => setForm({ ...form, baseUrl: e.target.value })}
      />
      <input
        aria-label="API key"
        type="password"
        placeholder={form.id ? 'API key (unchanged)' : 'API key (optional)'}
        className={field}
        value={apiKey}
        onChange={(e) => setApiKey(e.target.value)}
      />
      <label className="flex items-center gap-2 text-[13px] text-text-2">
        <input
          type="checkbox"
          checked={form.llamaExtensions}
          onChange={(e) => setForm({ ...form, llamaExtensions: e.target.checked })}
          className="accent-[var(--accent)]"
        />
        It's llama.cpp's llama-server (enables thinking switches and prompt progress)
      </label>
      {status && (
        <p className={`text-[12.5px] ${status.ok ? 'text-text-2' : 'text-danger'}`}>
          {status.text}
        </p>
      )}
      <div className="flex justify-end gap-2 pt-1">
        <Button size="sm" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button size="sm" onClick={() => void test()}>
          Test
        </Button>
        <Button
          size="sm"
          variant="primary"
          disabled={!form.name.trim() || !form.baseUrl.trim()}
          onClick={() => void save()}
        >
          Save
        </Button>
      </div>
    </div>
  );
}
