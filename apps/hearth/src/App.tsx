import { useEffect } from 'react';
import { Sidebar, newConversation } from './components/Sidebar';
import { TitleBar } from './components/TitleBar';
import { backend } from './lib/backend';
import { applyTheme, resolveTheme, watchSystemTheme } from './lib/theme';
import { ChatScreen } from './screens/ChatScreen';
import { CodeScreen } from './screens/CodeScreen';
import { ModelsScreen } from './screens/ModelsScreen';
import { SettingsScreen } from './screens/SettingsScreen';
import { useApp } from './stores/app';
import { useChat } from './stores/chat';

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
  useConversations();
  useShortcuts();

  return (
    <div className="hearth-app flex h-full bg-bg text-text" data-screen={screen}>
      {!collapsed && <Sidebar />}
      <div className="flex min-w-0 flex-1 flex-col">
        <TitleBar />
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

/** Keeps the sidebar lists, "working…" spinners and "needs you" dots in step with Rust. */
function useConversations() {
  useEffect(() => {
    const chat = useChat.getState();
    void chat.loadLists();
    void chat.loadProjects();
    void backend()
      .runsActive()
      .then((runs) => runs.forEach((run) => useChat.getState().setRunState(run)));
    const stopSessions = backend().onSessionsChanged(() => void useChat.getState().loadLists());
    const stopRuns = backend().onRunState((info) => useChat.getState().setRunState(info));
    return () => {
      stopSessions();
      stopRuns();
    };
  }, []);
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
        newConversation(app.mode);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
}
