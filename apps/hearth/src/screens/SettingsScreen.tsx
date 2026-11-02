import { Monitor, Moon, SquareTerminal, Sun, Terminal } from 'lucide-react';
import { type ReactNode, useEffect, useState } from 'react';
import { ConnectionsSettings } from '../components/ConnectionsSettings';
import { McpSettings } from '../components/McpSettings';
import { Button } from '../components/ui/Button';
import { Segmented } from '../components/ui/Segmented';
import { type WebKeys, backend, errorMessage } from '../lib/backend';
import type {
  SearchProvider,
  Settings,
  ShellKind,
  ShellsInfo,
  SystemInfo,
  ThemeSetting,
} from '../lib/types';
import { useApp } from '../stores/app';

export function SettingsScreen() {
  const theme = useApp((s) => s.settings.theme);
  const [info, setInfo] = useState<SystemInfo | null>(null);

  useEffect(() => {
    let live = true;
    backend()
      .systemInfo()
      .then((next) => live && setInfo(next))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto max-w-[720px] px-8 pb-16 pt-6">
        <h1 className="text-[22px] font-semibold tracking-[-0.018em]">Settings</h1>

        <Section title="Appearance">
          <Row label="Theme" hint="System follows Windows' light or dark mode.">
            <Segmented<ThemeSetting>
              label="Theme"
              className="w-[300px]"
              value={theme}
              onChange={(next) => {
                useApp
                  .getState()
                  .updateSettings({ theme: next })
                  .catch((err: unknown) => console.error('could not save the theme', err));
              }}
              options={[
                { value: 'system', label: 'System', icon: <Monitor /> },
                { value: 'light', label: 'Light', icon: <Sun /> },
                { value: 'dark', label: 'Dark', icon: <Moon /> },
              ]}
            />
          </Row>
        </Section>

        <Section title="Models">
          <ConnectionsSettings />
        </Section>

        <Section title="Coding">
          <ShellSetting />
        </Section>

        <Section title="Web">
          <WebSettings />
        </Section>

        <Section title="MCP servers">
          <McpSettings />
        </Section>

        <Section title="About">
          <Row label="Hearth">{info?.appVersion ?? '…'}</Row>
          <Row label="WebView2">{info?.webviewVersion ?? '…'}</Row>
          <Row label="Data folder">
            <span className="break-all font-mono text-[12px] select-text">
              {info?.dataDir ?? '…'}
            </span>
          </Row>
        </Section>
      </div>
    </div>
  );
}

/** Which shell the agent runs commands in, and what this PC has. */
function ShellSetting() {
  const shell = useApp((s) => s.settings.code.shell);
  const [found, setFound] = useState<ShellsInfo | null>(null);
  useEffect(() => {
    let live = true;
    backend()
      .shellsInfo()
      .then((next) => live && setFound(next))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);
  const path = shell === 'git_bash' ? found?.gitBash : found?.powershell;
  const hint = !found
    ? 'Looking for shells…'
    : path
      ? path
      : shell === 'git_bash'
        ? 'Git Bash isn’t installed (it comes with Git for Windows), so PowerShell is used.'
        : 'PowerShell wasn’t found, so Git Bash is used.';
  return (
    <Row label="Shell for commands" hint={hint}>
      <Segmented<ShellKind>
        label="Shell for commands"
        className="w-[240px]"
        value={shell}
        onChange={(next) => {
          useApp
            .getState()
            .updateSettings({ code: { shell: next } })
            .catch((err: unknown) => console.error('could not save the shell', err));
        }}
        options={[
          { value: 'git_bash', label: 'Git Bash', icon: <SquareTerminal /> },
          { value: 'powershell', label: 'PowerShell', icon: <Terminal /> },
        ]}
      />
    </Row>
  );
}

const PROVIDERS: { value: SearchProvider; label: string; hint: string }[] = [
  {
    value: 'duckduckgo',
    label: 'DuckDuckGo',
    hint: 'No key needed. After many searches it may stop answering for a while.',
  },
  {
    value: 'brave',
    label: 'Brave Search',
    hint: 'Needs an API key (free plan) from api-dashboard.search.brave.com.',
  },
  { value: 'tavily', label: 'Tavily', hint: 'Needs an API key from tavily.com.' },
  {
    value: 'searxng',
    label: 'SearXNG',
    hint: 'Your own SearXNG server, with JSON results turned on.',
  },
];

const FIELD =
  'h-8 rounded-control bg-surface-2 px-2.5 text-[13px] outline-none ring-1 ring-line focus:ring-accent select-text';

type TestState =
  | { state: 'idle' }
  | { state: 'running' }
  | { state: 'ok'; count: number }
  | { state: 'error'; message: string };

/** Where chats and the agent search, its key or server, and whether local pages may be read. */
function WebSettings() {
  const web = useApp((s) => s.settings.web);
  const [keys, setKeys] = useState<WebKeys | null>(null);
  const [test, setTest] = useState<TestState>({ state: 'idle' });
  useEffect(() => {
    let live = true;
    backend()
      .webKeys()
      .then((next) => live && setKeys(next))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);
  const save = (patch: Partial<Settings['web']>) => {
    setTest({ state: 'idle' });
    useApp
      .getState()
      .updateSettings({ web: patch })
      .catch((err: unknown) => console.error('could not save web settings', err));
  };
  const provider = PROVIDERS.find((p) => p.value === web.provider) ?? PROVIDERS[0]!;
  const runTest = async () => {
    setTest({ state: 'running' });
    try {
      const hits = await backend().webTest();
      setTest({ state: 'ok', count: hits.length });
    } catch (err) {
      setTest({ state: 'error', message: errorMessage(err) });
    }
  };
  return (
    <>
      <Row label="Search with" hint={provider.hint}>
        <select
          aria-label="Search with"
          value={web.provider}
          onChange={(e) => save({ provider: e.target.value as SearchProvider })}
          className={`${FIELD} w-[180px]`}
        >
          {PROVIDERS.map((p) => (
            <option key={p.value} value={p.value}>
              {p.label}
            </option>
          ))}
        </select>
      </Row>
      {(web.provider === 'brave' || web.provider === 'tavily') && (
        <ApiKeyRow
          provider={web.provider}
          label={provider.label}
          saved={keys?.[web.provider] ?? false}
          onSaved={(next) => {
            setKeys(next);
            setTest({ state: 'idle' });
          }}
        />
      )}
      {web.provider === 'searxng' && (
        <Row label="SearXNG address" hint="For example http://localhost:8888">
          <input
            aria-label="SearXNG address"
            placeholder="http://localhost:8888"
            defaultValue={web.searxngUrl ?? ''}
            onBlur={(e) => save({ searxngUrl: e.target.value.trim() || null })}
            className={`${FIELD} w-[240px] font-mono`}
          />
        </Row>
      )}
      <Row
        label="Read pages on this PC and your network"
        hint="Off keeps web tools on the internet, so a web page can't point them at your router or local servers. Off, pages are also read without your proxy; turn this on if your network needs one."
      >
        <input
          type="checkbox"
          aria-label="Read pages on this PC and your network"
          checked={web.allowLocal}
          onChange={(e) => save({ allowLocal: e.target.checked })}
          className="size-4 accent-[var(--accent)]"
        />
      </Row>
      <Row
        label="Check the search"
        hint={
          test.state === 'ok'
            ? `It works: ${test.count} result${test.count === 1 ? '' : 's'}.`
            : test.state === 'error'
              ? test.message
              : 'Runs one search with these settings.'
        }
      >
        <Button
          size="sm"
          variant="secondary"
          disabled={test.state === 'running'}
          onClick={() => void runTest()}
        >
          {test.state === 'running' ? 'Searching…' : 'Test search'}
        </Button>
      </Row>
    </>
  );
}

function ApiKeyRow({
  provider,
  label,
  saved,
  onSaved,
}: {
  provider: 'brave' | 'tavily';
  label: string;
  saved: boolean;
  onSaved(keys: WebKeys): void;
}) {
  const [key, setKey] = useState('');
  const store = (value: string) => {
    void backend()
      .webKeySet(provider, value)
      .then((next) => {
        setKey('');
        onSaved(next);
      })
      .catch((err: unknown) => console.error('could not save the key', err));
  };
  return (
    <Row
      label={`${label} API key`}
      hint={saved ? 'Saved on this PC.' : 'Kept in Hearth’s database on this PC.'}
    >
      <div className="flex items-center gap-2">
        <input
          aria-label={`${label} API key`}
          type="password"
          placeholder={saved ? '••••••••' : 'Paste the key'}
          value={key}
          onChange={(e) => setKey(e.target.value)}
          className={`${FIELD} w-[200px]`}
        />
        <Button size="sm" variant="secondary" disabled={!key.trim()} onClick={() => store(key)}>
          Save
        </Button>
        {saved && (
          <Button size="sm" variant="ghost" onClick={() => store('')}>
            Remove
          </Button>
        )}
      </div>
    </Row>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="mt-8">
      <h2 className="mb-2 text-[12px] font-medium uppercase tracking-[0.06em] text-text-3">
        {title}
      </h2>
      <div className="divide-y divide-line rounded-card bg-surface shadow-float ring-1 ring-line">
        {children}
      </div>
    </section>
  );
}

function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="flex items-center gap-6 px-5 py-3.5">
      <div className="min-w-0 flex-1">
        <p className="text-[14px] font-medium">{label}</p>
        {hint && <p className="mt-0.5 text-[12.5px] text-text-2">{hint}</p>}
      </div>
      <div className="shrink-0 text-[13px] text-text-2">{children}</div>
    </div>
  );
}
