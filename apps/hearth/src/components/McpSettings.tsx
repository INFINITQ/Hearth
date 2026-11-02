// Settings → MCP servers: add, import, turn on and off, and see each server's state and tools.

import {
  ChevronRight,
  FileJson,
  Globe,
  Plus,
  RotateCw,
  SquareTerminal,
  Trash2,
} from 'lucide-react';
import { useEffect, useState } from 'react';
import { backend, errorMessage } from '../lib/backend';
import { cn } from '../lib/cn';
import type { McpServer, McpServerConfig } from '../lib/types';
import { Button, IconButton } from './ui/Button';
import { Segmented } from './ui/Segmented';

const FIELD =
  'w-full rounded-control bg-surface px-2.5 text-[13px] outline-none ring-1 ring-line focus:ring-accent select-text';

/** A server as the form edits it (lists as lines). */
type Draft = {
  id: string;
  name: string;
  enabled: boolean;
  type: 'stdio' | 'http';
  command: string;
  args: string;
  env: string;
  cwd: string;
  url: string;
  headers: string;
};

const NEW: Draft = {
  id: '',
  name: '',
  enabled: true,
  type: 'stdio',
  command: '',
  args: '',
  env: '',
  cwd: '',
  url: '',
  headers: '',
};

function lines(text: string): string[] {
  return text
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean);
}

/** `KEY=value` (or `Name: value`) lines as a map. */
function pairs(text: string, separator: '=' | ':'): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of lines(text)) {
    const at = line.indexOf(separator);
    if (at > 0) out[line.slice(0, at).trim()] = line.slice(at + 1).trim();
  }
  return out;
}

function draftOf(server: McpServer): Draft {
  return server.type === 'stdio'
    ? {
        ...NEW,
        id: server.id,
        name: server.name,
        enabled: server.enabled,
        command: server.command,
        args: server.args.join('\n'),
        env: Object.entries(server.env)
          .map(([k, v]) => `${k}=${v}`)
          .join('\n'),
        cwd: server.cwd ?? '',
      }
    : {
        ...NEW,
        id: server.id,
        name: server.name,
        enabled: server.enabled,
        type: 'http',
        url: server.url,
        headers: Object.entries(server.headers)
          .map(([k, v]) => `${k}: ${v}`)
          .join('\n'),
      };
}

function configOf(draft: Draft): McpServerConfig {
  const base = { id: draft.id, name: draft.name.trim(), enabled: draft.enabled };
  return draft.type === 'stdio'
    ? {
        ...base,
        type: 'stdio',
        command: draft.command.trim(),
        args: lines(draft.args),
        env: pairs(draft.env, '='),
        cwd: draft.cwd.trim() || null,
      }
    : { ...base, type: 'http', url: draft.url.trim(), headers: pairs(draft.headers, ':') };
}

/** MCP servers give the coding agent more tools. */
export function McpSettings() {
  const [servers, setServers] = useState<McpServer[]>([]);
  const [editing, setEditing] = useState<Draft | null>(null);
  const [importing, setImporting] = useState(false);

  useEffect(() => {
    let live = true;
    backend()
      .mcpServers()
      .then((next) => live && setServers(next))
      .catch(() => {});
    const stop = backend().onMcp((next) => live && setServers(next));
    return () => {
      live = false;
      stop();
    };
  }, []);

  const busy = editing !== null || importing;
  return (
    <div className="px-5 py-3.5">
      <div className="flex items-center justify-between gap-4">
        <div>
          <p className="text-[14px] font-medium">Servers</p>
          <p className="mt-0.5 text-[12.5px] text-text-2">
            They give the coding agent more tools (files, GitHub, databases…). It asks before using
            one, unless you always allow it.
          </p>
        </div>
        {!busy && (
          <div className="flex shrink-0 gap-1.5">
            <Button size="sm" variant="ghost" onClick={() => setImporting(true)}>
              <FileJson /> Import
            </Button>
            <Button size="sm" onClick={() => setEditing(NEW)}>
              <Plus /> Add
            </Button>
          </div>
        )}
      </div>
      {servers.length > 0 && (
        <ul className="mt-3 space-y-1.5">
          {servers.map((server) => (
            <ServerRow
              key={server.id}
              server={server}
              onEdit={() => setEditing(draftOf(server))}
              onChange={setServers}
            />
          ))}
        </ul>
      )}
      {editing && (
        <ServerForm
          initial={editing}
          onDone={(next) => {
            setEditing(null);
            if (next) setServers(next);
          }}
        />
      )}
      {importing && (
        <ImportForm
          onDone={(next) => {
            setImporting(false);
            if (next) setServers(next);
          }}
        />
      )}
    </div>
  );
}

function ServerRow({
  server,
  onEdit,
  onChange,
}: {
  server: McpServer;
  onEdit(): void;
  onChange(servers: McpServer[]): void;
}) {
  const [open, setOpen] = useState(false);
  const { status } = server;
  const summary =
    status.state === 'ready'
      ? `Ready · ${status.tools.length} tool${status.tools.length === 1 ? '' : 's'}`
      : status.state === 'starting'
        ? 'Starting…'
        : status.state === 'failed'
          ? 'Couldn’t start'
          : 'Off';
  const target = server.type === 'stdio' ? [server.command, ...server.args].join(' ') : server.url;
  const act = (call: Promise<McpServer[]>) => void call.then(onChange).catch(() => {});
  return (
    <li className="rounded-control bg-surface-2" data-mcp={server.name} data-state={status.state}>
      <div className="flex items-center gap-2 py-1.5 pl-3 pr-1.5">
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
          className="flex min-w-0 flex-1 items-center gap-2.5 text-left"
        >
          <span
            className={cn(
              'size-2 shrink-0 rounded-full',
              status.state === 'ready' && 'bg-success',
              status.state === 'starting' && 'animate-pulse bg-accent',
              status.state === 'failed' && 'bg-danger',
              status.state === 'off' && 'bg-text-3/50',
            )}
          />
          <span className="min-w-0 flex-1">
            <span className="flex items-baseline gap-2">
              <span className="truncate text-[13px] font-medium">{server.name}</span>
              <span
                className={cn(
                  'shrink-0 text-[12px]',
                  status.state === 'failed' ? 'text-danger' : 'text-text-2',
                )}
              >
                {summary}
              </span>
            </span>
            <span className="block truncate font-mono text-[11.5px] text-text-3">{target}</span>
          </span>
          <ChevronRight
            className={cn(
              'size-3.5 shrink-0 text-text-3 transition-transform duration-150',
              open && 'rotate-90',
            )}
          />
        </button>
        <input
          type="checkbox"
          aria-label={`Use ${server.name}`}
          title={server.enabled ? 'On' : 'Off'}
          checked={server.enabled}
          onChange={(e) => act(backend().mcpSetEnabled(server.id, e.target.checked))}
          className="mx-1 size-4 accent-[var(--accent)]"
        />
        <Button size="sm" variant="ghost" onClick={onEdit}>
          Edit
        </Button>
        <IconButton
          label={`Restart ${server.name}`}
          onClick={() => act(backend().mcpRestart(server.id))}
        >
          <RotateCw />
        </IconButton>
        <IconButton
          label={`Remove ${server.name}`}
          onClick={() => act(backend().mcpRemove(server.id))}
        >
          <Trash2 />
        </IconButton>
      </div>
      {open && (
        <div className="border-t border-line px-3 py-2.5 text-[12.5px]">
          {status.state === 'ready' &&
            (status.tools.length > 0 ? (
              <ul className="space-y-1" aria-label={`${server.name} tools`}>
                {status.tools.map((tool) => (
                  <li key={tool.name}>
                    <span className="font-mono text-text">{tool.tool}</span>
                    {tool.description && <span className="text-text-2"> · {tool.description}</span>}
                  </li>
                ))}
              </ul>
            ) : (
              <p className="text-text-2">It offers no tools.</p>
            ))}
          {status.state === 'failed' && (
            <pre className="whitespace-pre-wrap break-words font-mono text-[12px] text-danger select-text">
              {status.error}
            </pre>
          )}
          {status.state === 'starting' && (
            <p className="text-text-2">Starting… Servers run with `npx` may download first.</p>
          )}
          {status.state === 'off' && <p className="text-text-2">Turned off.</p>}
          {server.log.length > 0 && (
            <details className="mt-2">
              <summary className="cursor-default text-text-3">What it printed</summary>
              <pre className="mt-1 max-h-[160px] overflow-auto whitespace-pre-wrap break-words font-mono text-[11.5px] text-text-2 select-text">
                {server.log.join('\n')}
              </pre>
            </details>
          )}
        </div>
      )}
    </li>
  );
}

function ServerForm({
  initial,
  onDone,
}: {
  initial: Draft;
  onDone(servers: McpServer[] | null): void;
}) {
  const [draft, setDraft] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const set = (patch: Partial<Draft>) => setDraft({ ...draft, ...patch });
  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      onDone(await backend().mcpSave(configOf(draft)));
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setSaving(false);
    }
  };
  return (
    <div
      role="group"
      aria-label={initial.id ? 'Edit MCP server' : 'New MCP server'}
      className="mt-3 space-y-2.5 rounded-card bg-surface-2 p-3.5"
    >
      <input
        aria-label="Server name"
        placeholder="Name (e.g. GitHub)"
        className={`${FIELD} h-8`}
        value={draft.name}
        onChange={(e) => set({ name: e.target.value })}
      />
      <Segmented<'stdio' | 'http'>
        label="How Hearth reaches it"
        className="w-[240px]"
        value={draft.type}
        onChange={(type) => set({ type })}
        options={[
          { value: 'stdio', label: 'Command', icon: <SquareTerminal /> },
          { value: 'http', label: 'URL', icon: <Globe /> },
        ]}
      />
      {draft.type === 'stdio' ? (
        <>
          <input
            aria-label="Command"
            placeholder="Command (e.g. npx, uvx, node)"
            className={`${FIELD} h-8 font-mono`}
            value={draft.command}
            onChange={(e) => set({ command: e.target.value })}
          />
          <textarea
            aria-label="Arguments"
            placeholder={
              'Arguments, one per line, e.g.\n-y\n@modelcontextprotocol/server-filesystem'
            }
            rows={3}
            className={`${FIELD} resize-y py-1.5 font-mono`}
            value={draft.args}
            onChange={(e) => set({ args: e.target.value })}
          />
          <textarea
            aria-label="Environment variables"
            placeholder="Environment variables, KEY=value, one per line"
            rows={2}
            className={`${FIELD} resize-y py-1.5 font-mono`}
            value={draft.env}
            onChange={(e) => set({ env: e.target.value })}
          />
          <input
            aria-label="Start in folder"
            placeholder="Folder to start in (optional)"
            className={`${FIELD} h-8 font-mono`}
            value={draft.cwd}
            onChange={(e) => set({ cwd: e.target.value })}
          />
        </>
      ) : (
        <>
          <input
            aria-label="URL"
            placeholder="https://example.com/mcp"
            className={`${FIELD} h-8 font-mono`}
            value={draft.url}
            onChange={(e) => set({ url: e.target.value })}
          />
          <textarea
            aria-label="Headers"
            placeholder="Headers, Name: value, one per line (e.g. Authorization: Bearer …)"
            rows={2}
            className={`${FIELD} resize-y py-1.5 font-mono`}
            value={draft.headers}
            onChange={(e) => set({ headers: e.target.value })}
          />
        </>
      )}
      {error && <p className="text-[12.5px] text-danger">{error}</p>}
      <div className="flex justify-end gap-2 pt-1">
        <Button size="sm" variant="ghost" onClick={() => onDone(null)}>
          Cancel
        </Button>
        <Button size="sm" variant="primary" disabled={saving} onClick={() => void save()}>
          {saving ? 'Saving…' : 'Save'}
        </Button>
      </div>
    </div>
  );
}

function ImportForm({ onDone }: { onDone(servers: McpServer[] | null): void }) {
  const [text, setText] = useState('');
  const [error, setError] = useState<string | null>(null);
  const run = async () => {
    setError(null);
    try {
      onDone(await backend().mcpImport(text));
    } catch (err) {
      setError(errorMessage(err));
    }
  };
  return (
    <div
      role="group"
      aria-label="Import MCP servers"
      className="mt-3 space-y-2.5 rounded-card bg-surface-2 p-3.5"
    >
      <p className="text-[12.5px] text-text-2">
        Paste a config from Claude Desktop, Cursor or VS Code: the part with
        <code className="mx-1 font-mono">"mcpServers"</code>. Servers with the same name are
        replaced.
      </p>
      <textarea
        aria-label="MCP config JSON"
        placeholder={
          '{\n  "mcpServers": {\n    "files": { "command": "npx", "args": ["-y", "…"] }\n  }\n}'
        }
        rows={7}
        className={`${FIELD} resize-y py-1.5 font-mono`}
        value={text}
        onChange={(e) => setText(e.target.value)}
      />
      {error && <p className="text-[12.5px] text-danger">{error}</p>}
      <div className="flex justify-end gap-2">
        <Button size="sm" variant="ghost" onClick={() => onDone(null)}>
          Cancel
        </Button>
        <Button size="sm" variant="primary" disabled={!text.trim()} onClick={() => void run()}>
          Import
        </Button>
      </div>
    </div>
  );
}
