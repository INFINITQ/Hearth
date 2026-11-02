import { useEffect } from 'react';
import { Sidebar } from './components/Sidebar';
import { TitleBar } from './components/TitleBar';
import { backend } from './lib/backend';
import { applyTheme, resolveTheme, watchSystemTheme } from './lib/theme';
import { ChatScreen } from './screens/ChatScreen';
import { CodeScreen } from './screens/CodeScreen';
import { ModelsScreen } from './screens/ModelsScreen';
import { SettingsScreen } from './screens/SettingsScreen';
import { type Screen, useApp } from './stores/app';

// Pages with their own heading leave the title bar empty; chats will show their title here.
const TITLES: Record<Screen, string> = {
  chat: '',
  code: '',
  models: '',
  settings: '',
};

export function App() {
  const screen = useApp((s) => s.screen);
  const theme = useApp((s) => s.settings.theme);
  const systemDark = useApp((s) => s.systemDark);
  const collapsed = useApp((s) => s.settings.sidebarCollapsed);
  const resolved = resolveTheme(theme, systemDark);

  useEffect(() => {
    applyTheme(resolved);
    backend()
      .setWindowDark(resolved === 'dark')
      .catch(() => {});
  }, [resolved]);
  useEffect(() => watchSystemTheme((dark) => useApp.getState().setSystemDark(dark)), []);
  useEffect(
    () => backend().onSettings((settings) => useApp.getState().applySettings(settings)),
    [],
  );
  useShortcuts();

  return (
    <div className="hearth-app flex h-full bg-bg text-text" data-screen={screen}>
      {!collapsed && <Sidebar />}
      <div className="flex min-w-0 flex-1 flex-col">
        <TitleBar title={TITLES[screen]} />
        <main className="flex min-h-0 flex-1 flex-col">
          {screen === 'chat' && <ChatScreen />}
          {screen === 'code' && <CodeScreen />}
          {screen === 'models' && <ModelsScreen />}
          {screen === 'settings' && <SettingsScreen />}
        </main>
      </div>
    </div>
  );
}

/** App-wide keyboard shortcuts (WebView2's own browser shortcuts are switched off). */
function useShortcuts() {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!event.ctrlKey || event.altKey || event.metaKey) return;
      const app = useApp.getState();
      const key = event.key.toLowerCase();
      if (key === 'b' && !event.shiftKey) {
        event.preventDefault();
        app.toggleSidebar();
      } else if (key === ',') {
        event.preventDefault();
        app.go('settings');
      } else if (key === 'n' && !event.shiftKey) {
        event.preventDefault();
        app.setMode(app.mode);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
}
