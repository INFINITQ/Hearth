import {
  Boxes,
  ChevronRight,
  Cpu,
  Download,
  FolderPlus,
  LoaderCircle,
  RefreshCw,
  TriangleAlert,
  Wrench,
  X,
} from 'lucide-react';
import { type ReactNode, useEffect, useState } from 'react';
import { Button, IconButton } from '../components/ui/Button';
import { backend } from '../lib/backend';
import { cn } from '../lib/cn';
import type { LocalModel, ModelPreset, ModelStatus, RuntimeOverview } from '../lib/types';
import { useApp } from '../stores/app';
import { formatSize, formatTokens, useModels } from '../stores/models';

/** Models: llama.cpp, the folders with models, and the models found there. */
export function ModelsScreen() {
  const overview = useModels((s) => s.overview);
  const refreshing = useModels((s) => s.refreshing);

  useEffect(() => {
    void useModels.getState().load(true);
    void useModels.getState().readTelemetry();
    const timer = setInterval(() => void useModels.getState().readTelemetry(), 2000);
    return () => clearInterval(timer);
  }, []);

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto max-w-[820px] px-8 pb-16 pt-6">
        <div className="flex items-start gap-4">
          <div className="min-w-0 flex-1">
            <h1 className="text-[22px] font-semibold tracking-[-0.018em]">Models</h1>
            <p className="mt-1 text-[14px] text-text-2">
              Hearth runs models on your PC with llama.cpp. Nothing leaves your machine.
            </p>
          </div>
          <MemoryMeters />
        </div>

        {!overview ? (
          <p className="mt-8 text-[13px] text-text-3">Looking for llama.cpp and models…</p>
        ) : (
          <>
            <ServerCard />
            <FoldersCard />
            <section className="mt-8" aria-label="Models found">
              <div className="mb-2 flex items-center gap-2">
                <h2 className="text-[12px] font-medium uppercase tracking-[0.06em] text-text-3">
                  Models {overview.models.length > 0 && `(${overview.models.length})`}
                </h2>
                <div className="flex-1" />
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={refreshing}
                  onClick={() => void useModels.getState().load(true)}
                >
                  <RefreshCw className={cn(refreshing && 'animate-spin')} />
                  Rescan
                </Button>
              </div>
              {overview.models.length === 0 ? (
                <div className="rounded-card bg-surface px-5 py-6 text-[13.5px] text-text-2 shadow-float ring-1 ring-line">
                  No models found yet. Download a GGUF model (see the models guide), then add its
                  folder above.
                </div>
              ) : (
                <div className="space-y-2.5">
                  {overview.models.map((model) => (
                    <ModelRow
                      key={model.id}
                      model={model}
                      status={overview.status.find((s) => s.modelId === model.id)}
                    />
                  ))}
                </div>
              )}
            </section>
          </>
        )}
      </div>
    </div>
  );
}

function Meter({ label, used, total }: { label: string; used: number; total: number }) {
  const share = total > 0 ? Math.min(1, used / total) : 0;
  return (
    <div
      className="w-[132px]"
      title={`${label}: ${(used / 1024).toFixed(1)} of ${(total / 1024).toFixed(1)} GB in use`}
    >
      <div className="flex items-baseline justify-between text-[11.5px]">
        <span className="font-medium text-text-2">{label}</span>
        <span className="tabular-nums text-text-3">
          {(used / 1024).toFixed(1)} / {(total / 1024).toFixed(0)} GB
        </span>
      </div>
      <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-surface-3">
        <div
          className={cn('h-full rounded-full', share > 0.9 ? 'bg-danger' : 'bg-accent')}
          style={{ width: `${share * 100}%` }}
        />
      </div>
    </div>
  );
}

function MemoryMeters() {
  const telemetry = useModels((s) => s.telemetry);
  if (!telemetry) return null;
  const gpu = telemetry.gpus[0];
  return (
    <div className="flex shrink-0 gap-4 pt-1.5" aria-label="Memory in use">
      {gpu && <Meter label="VRAM" used={gpu.usedMib} total={gpu.totalMib} />}
      <Meter label="RAM" used={telemetry.ramUsedMib} total={telemetry.ramTotalMib} />
    </div>
  );
}

function Card({
  title,
  children,
  actions,
}: {
  title: string;
  children: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <section className="mt-8">
      <div className="mb-2 flex items-center gap-2">
        <h2 className="text-[12px] font-medium uppercase tracking-[0.06em] text-text-3">{title}</h2>
        <div className="flex-1" />
        {actions}
      </div>
      <div className="rounded-card bg-surface shadow-float ring-1 ring-line">{children}</div>
    </section>
  );
}

function ServerCard() {
  const overview = useModels((s) => s.overview)!;
  const refreshing = useModels((s) => s.refreshing);
  const server = overview.server;
  if (!server) {
    return (
      <Card title="llama.cpp">
        <div className="p-5">
          <div className="flex items-start gap-3">
            <div className="grid size-10 shrink-0 place-items-center rounded-control bg-accent-soft text-accent">
              <Boxes className="size-5 stroke-[1.75]" />
            </div>
            <div className="min-w-0 flex-1">
              <h3 className="text-[15px] font-semibold">Set up llama.cpp</h3>
              <p className="mt-0.5 text-[13px] text-text-2 select-text">{overview.serverError}</p>
            </div>
          </div>
          <ol className="mt-5 space-y-3.5">
            <Step icon={<Wrench />} n={1} title="Install llama.cpp">
              The Windows CUDA 13 build from github.com/ggml-org/llama.cpp/releases: both zips into
              one folder, added to PATH.
            </Step>
            <Step icon={<Download />} n={2} title="Download a GGUF model">
              For an RTX 5070: Qwen3.6-35B-A3B for coding, Gemma 4 26B-A4B for chat.
            </Step>
            <Step icon={<FolderPlus />} n={3} title="Add its folder below">
              Hearth finds the models, picks good settings, and loads them when you need them.
            </Step>
          </ol>
          <div className="mt-5 flex gap-2">
            <Button variant="primary" onClick={() => void useModels.getState().chooseServer()}>
              Choose llama-server.exe…
            </Button>
            <Button
              variant="ghost"
              disabled={refreshing}
              onClick={() => void useModels.getState().load(true)}
            >
              Check again
            </Button>
          </div>
        </div>
      </Card>
    );
  }
  const gpu = server.devices.find((d) => d.id.startsWith('CUDA') || d.id.startsWith('Vulkan'));
  return (
    <Card title="llama.cpp">
      <div className="flex items-center gap-3 px-5 py-4">
        <span className="size-2 shrink-0 rounded-full bg-success" aria-hidden />
        <div className="min-w-0 flex-1">
          <p className="text-[14px] font-medium">
            llama-server <span className="font-normal text-text-2">{server.version}</span>
          </p>
          <p className="mt-0.5 truncate text-[12.5px] text-text-3" title={server.path}>
            {gpu ? `${gpu.name} · ${(gpu.totalMib / 1024).toFixed(0)} GB` : 'No GPU found'} ·{' '}
            {server.path}
          </p>
        </div>
        <Button size="sm" onClick={() => void useModels.getState().chooseServer()}>
          Change…
        </Button>
      </div>
      {server.warnings.map((warning) => (
        <p
          key={warning}
          className="mx-5 mb-4 flex items-start gap-2 rounded-control bg-accent-soft px-3 py-2 text-[12.5px] text-accent-text"
        >
          <TriangleAlert className="mt-0.5 size-3.5 shrink-0" />
          {warning}
        </p>
      ))}
    </Card>
  );
}

const NO_FOLDERS: RuntimeOverview['folders'] = [];

function FoldersCard() {
  const folders = useModels((s) => s.overview?.folders ?? NO_FOLDERS);
  return (
    <Card
      title="Model folders"
      actions={
        <Button size="sm" variant="ghost" onClick={() => void useModels.getState().addFolder()}>
          <FolderPlus />
          Add folder…
        </Button>
      }
    >
      {folders.length === 0 ? (
        <p className="px-5 py-4 text-[13px] text-text-3">
          No folders yet. Add the folder you download models to, like D:\Models.
        </p>
      ) : (
        <ul className="divide-y divide-line">
          {folders.map((folder) => (
            <li key={folder.path} className="flex items-center gap-3 px-5 py-2.5">
              <span
                className="min-w-0 flex-1 truncate font-mono text-[12.5px] text-text-2"
                title={folder.path}
              >
                {folder.path}
              </span>
              {folder.automatic && <span className="text-[11.5px] text-text-3">LM Studio</span>}
              {!folder.exists && <span className="text-[11.5px] text-danger">Missing</span>}
              {!folder.automatic && (
                <IconButton
                  label={`Remove ${folder.path}`}
                  onClick={() => void useModels.getState().removeFolder(folder.path)}
                >
                  <X />
                </IconButton>
              )}
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}

function StatusPill({ status, busy }: { status: ModelStatus | undefined; busy: boolean }) {
  const [now, setNow] = useState(Date.now());
  const loading = busy || status?.state === 'loading';
  useEffect(() => {
    if (!loading) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [loading]);
  if (loading) {
    const since = status?.state === 'loading' ? status.sinceMs : now;
    return (
      <span
        className="flex items-center gap-1.5 text-[12.5px] text-accent-text"
        data-state="loading"
      >
        <LoaderCircle className="size-3.5 animate-spin" />
        Loading… {Math.max(0, Math.round((now - since) / 1000))}s
      </span>
    );
  }
  if (status?.state === 'ready') {
    return (
      <span className="flex items-center gap-1.5 text-[12.5px] text-success" data-state="ready">
        <span className="size-1.5 rounded-full bg-success" />
        Loaded
      </span>
    );
  }
  return (
    <span className="text-[12.5px] text-text-3" data-state={status?.state ?? 'idle'}>
      {status?.state === 'failed' ? 'Failed' : 'Not loaded'}
    </span>
  );
}

function ModelRow({ model, status }: { model: LocalModel; status: ModelStatus | undefined }) {
  const roles = useApp((s) => s.settings.models);
  const preset = useApp((s) => s.settings.runtime.presets[model.id]);
  const loadError = useModels((s) => s.errors[model.id]);
  const [busy, setBusy] = useState(false);
  const [open, setOpen] = useState(false);
  const [logs, setLogs] = useState<string[] | null>(null);
  const loaded = status?.state === 'ready' || status?.state === 'loading';
  const failure =
    loadError ?? (status?.state === 'failed' && status.error !== 'Unloaded.' ? status.error : null);
  const isRole = (role: 'chat' | 'code') => {
    const ref = roles[role];
    return ref?.kind === 'local' && ref.modelId === model.id;
  };
  const facts = [
    model.architecture,
    model.sizeLabel,
    formatSize(model.sizeBytes),
    model.contextLength ? `${formatTokens(model.contextLength)} context` : null,
    model.parts > 1 ? `${model.parts} parts` : null,
    model.mmproj ? 'vision' : null,
    model.tools ? 'tools' : null,
    model.thinking ? 'thinking' : null,
  ].filter(Boolean);

  const load = async () => {
    setBusy(true);
    await useModels.getState().loadModel(model.id);
    setBusy(false);
  };

  return (
    <article
      aria-label={model.name}
      data-model={model.id}
      className="rounded-card bg-surface px-5 py-4 shadow-float ring-1 ring-line"
    >
      <div className="flex items-start gap-3">
        <div className="grid size-9 shrink-0 place-items-center rounded-control bg-surface-2 text-text-2">
          <Cpu className="size-[18px] stroke-[1.75]" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h3 className="truncate text-[14.5px] font-semibold" title={model.path}>
              {model.name}
            </h3>
            {model.quant && (
              <span className="shrink-0 rounded-[5px] bg-surface-2 px-1.5 py-0.5 font-mono text-[11px] text-text-2">
                {model.quant}
              </span>
            )}
          </div>
          <p className="mt-0.5 truncate text-[12.5px] text-text-3">{facts.join(' · ')}</p>
        </div>
        <div className="flex shrink-0 items-center gap-3">
          <StatusPill status={status} busy={busy} />
          {loaded ? (
            <Button size="sm" onClick={() => void useModels.getState().unloadModel(model.id)}>
              Unload
            </Button>
          ) : (
            <Button
              size="sm"
              variant="primary"
              disabled={busy || model.missingParts > 0}
              onClick={() => void load()}
            >
              Load
            </Button>
          )}
        </div>
      </div>

      <div className="mt-3 flex items-center gap-2 pl-12">
        {(['chat', 'code'] as const).map((role) => (
          <button
            key={role}
            type="button"
            aria-pressed={isRole(role)}
            onClick={() => void useModels.getState().setRole(role, model.id)}
            className="h-7 rounded-full px-3 text-[12px] font-medium text-text-2 ring-1 ring-line transition-colors duration-150 hover:bg-surface-2 hover:text-text aria-pressed:bg-accent-soft aria-pressed:text-accent-text aria-pressed:ring-transparent"
          >
            {isRole(role) ? '✓ ' : ''}
            {role === 'chat' ? 'Chat model' : 'Coding model'}
          </button>
        ))}
        <div className="flex-1" />
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
          className="flex h-7 items-center gap-1 rounded-control px-2 text-[12.5px] text-text-2 hover:bg-surface-2 hover:text-text"
        >
          Settings
          <ChevronRight className={cn('size-3.5 transition-transform', open && 'rotate-90')} />
        </button>
      </div>

      {model.missingParts > 0 && (
        <p className="ml-12 mt-3 text-[12.5px] text-danger">
          {model.missingParts} of {model.parts} parts are missing. Download the rest into the same
          folder.
        </p>
      )}
      {failure && (
        <div
          role="alert"
          className="ml-12 mt-3 rounded-control bg-danger-soft px-3 py-2 text-[12.5px] text-danger"
        >
          <p className="select-text">{failure}</p>
          <button
            type="button"
            className="mt-1 font-medium underline-offset-2 hover:underline"
            onClick={async () => setLogs(logs ? null : await backend().modelLogs(model.id))}
          >
            {logs ? 'Hide the log' : 'Show the log'}
          </button>
          {logs && (
            <pre className="mt-2 max-h-[200px] overflow-auto whitespace-pre-wrap font-mono text-[11.5px] text-text-2 select-text">
              {logs.join('\n') || '(nothing logged)'}
            </pre>
          )}
        </div>
      )}
      {open && <PresetForm model={model} preset={preset} />}
    </article>
  );
}

const CONTEXTS = [8192, 16384, 32768, 65536, 131072, 262144];

function PresetForm({ model, preset }: { model: LocalModel; preset: ModelPreset | undefined }) {
  const current = preset ?? {};
  const save = (patch: ModelPreset) =>
    void useModels.getState().setPreset(model.id, { ...current, ...patch });
  const maxContext = model.contextLength ?? 262_144;
  const field =
    'h-8 rounded-control bg-surface-2 px-2.5 text-[12.5px] text-text outline-none ring-line focus:ring-1 select-text';
  return (
    <div className="ml-12 mt-3 grid grid-cols-2 gap-x-5 gap-y-3 border-t border-line pt-3.5 text-[12.5px]">
      <label className="flex flex-col gap-1">
        <span className="text-text-2">Context length</span>
        <select
          className={field}
          value={current.context ?? ''}
          onChange={(e) => save({ context: e.target.value ? Number(e.target.value) : null })}
        >
          <option value="">Hearth's choice ({formatTokens(Math.min(maxContext, 32768))})</option>
          {CONTEXTS.filter((c) => c <= maxContext).map((c) => (
            <option key={c} value={c}>
              {formatTokens(c)} tokens
            </option>
          ))}
        </select>
      </label>
      <label className="flex flex-col gap-1">
        <span className="text-text-2">KV cache</span>
        <select
          className={field}
          value={current.kvCache ?? ''}
          onChange={(e) => save({ kvCache: e.target.value || null })}
        >
          <option value="">Default (f16)</option>
          <option value="q8_0">q8_0 (half the memory)</option>
          <option value="q4_0">q4_0 (a quarter, less accurate)</option>
        </select>
      </label>
      <label className="flex flex-col gap-1">
        <span className="text-text-2">GPU layers</span>
        <input
          className={field}
          inputMode="numeric"
          placeholder="Automatic (fit to VRAM)"
          defaultValue={current.gpuLayers ?? ''}
          onBlur={(e) => save({ gpuLayers: e.target.value.trim() ? Number(e.target.value) : null })}
        />
      </label>
      <label className="flex flex-col gap-1">
        <span className="text-text-2">MoE layers kept in RAM</span>
        <input
          className={field}
          inputMode="numeric"
          placeholder={model.experts ? 'Automatic' : 'Not a mixture-of-experts model'}
          disabled={!model.experts}
          defaultValue={current.cpuMoe ?? ''}
          onBlur={(e) => save({ cpuMoe: e.target.value.trim() ? Number(e.target.value) : null })}
        />
      </label>
      <label className="col-span-2 flex flex-col gap-1">
        <span className="text-text-2">More llama-server arguments</span>
        <input
          className={cn(field, 'font-mono')}
          placeholder="e.g. --temp 0.6 --top-k 20"
          defaultValue={current.extraArgs ?? ''}
          onBlur={(e) => save({ extraArgs: e.target.value })}
        />
      </label>
      {model.mmproj && (
        <label className="col-span-2 flex items-center gap-2 text-text-2">
          <input
            type="checkbox"
            checked={current.vision ?? true}
            onChange={(e) => save({ vision: e.target.checked })}
          />
          Load the vision projector (lets the model see images; uses about 1 GB more)
        </label>
      )}
      <p className="col-span-2 text-[11.5px] text-text-3">
        Changes apply the next time the model loads.
      </p>
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
