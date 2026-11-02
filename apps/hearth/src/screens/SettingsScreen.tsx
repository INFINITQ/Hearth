import { Monitor, Moon, SquareTerminal, Sun, Terminal } from 'lucide-react';
import { type ReactNode, useEffect, useState } from 'react';
import { ConnectionsSettings } from '../components/ConnectionsSettings';
import { Segmented } from '../components/ui/Segmented';
import { backend } from '../lib/backend';
import type { ShellKind, ShellsInfo, SystemInfo, ThemeSetting } from '../lib/types';
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
