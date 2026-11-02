// Ctrl+K: every action and recent conversation, filtered as you type.

import {
  BookOpen,
  Boxes,
  Code2,
  FolderOpen,
  Globe,
  MessageSquare,
  Monitor,
  Moon,
  PanelLeft,
  Search,
  Settings,
  SquarePen,
  SquareTerminal,
  Sun,
} from 'lucide-react';
import { Dialog } from 'radix-ui';
import { type ReactNode, useEffect, useId, useMemo, useRef, useState } from 'react';
import { cn } from '../lib/cn';
import { useApp } from '../stores/app';
import { useChat } from '../stores/chat';
import { useTerminal } from '../stores/terminal';
import { newConversation } from './Sidebar';

type Command = {
  id: string;
  label: string;
  group: 'Actions' | 'Chats' | 'Coding sessions';
  icon: ReactNode;
  shortcut?: string;
  run(): void;
};

function actions(): Command[] {
  const app = useApp.getState();
  const chat = useChat.getState();
  const list: Command[] = [
    {
      id: 'new-chat',
      label: 'New chat',
      group: 'Actions',
      icon: <SquarePen />,
      shortcut: app.mode === 'chat' ? 'Ctrl+N' : undefined,
      run: () => newConversation('chat'),
    },
    {
      id: 'new-session',
      label: 'New coding session',
      group: 'Actions',
      icon: <Code2 />,
      shortcut: app.mode === 'code' ? 'Ctrl+N' : undefined,
      run: () => newConversation('code'),
    },
    {
      id: 'open-project',
      label: 'Open a project folder…',
      group: 'Actions',
      icon: <FolderOpen />,
      run: () => {
        app.go('code');
        void useChat.getState().pickProject();
      },
    },
    {
      id: 'web',
      label: chat.web ? 'Turn web search off' : 'Turn web search on',
      group: 'Actions',
      icon: <Globe />,
      run: () => {
        useChat.getState().setWeb(!useChat.getState().web);
        app.go('chat');
      },
    },
    {
      id: 'models',
      label: 'Models',
      group: 'Actions',
      icon: <Boxes />,
      run: () => app.go('models'),
    },
    {
      id: 'settings',
      label: 'Settings',
      group: 'Actions',
      icon: <Settings />,
      shortcut: 'Ctrl+,',
      run: () => app.go('settings'),
    },
    {
      id: 'guide',
      label: 'Guide: installing llama.cpp and models',
      group: 'Actions',
      icon: <BookOpen />,
      run: () => app.go('guide'),
    },
    {
      id: 'sidebar',
      label: app.settings.sidebarCollapsed ? 'Show the sidebar' : 'Hide the sidebar',
      group: 'Actions',
      icon: <PanelLeft />,
      shortcut: 'Ctrl+B',
      run: () => app.toggleSidebar(),
    },
    ...(['light', 'dark', 'system'] as const).map((theme) => ({
      id: `theme-${theme}`,
      label: `Theme: ${theme === 'system' ? 'like Windows' : theme}`,
      group: 'Actions' as const,
      icon: theme === 'light' ? <Sun /> : theme === 'dark' ? <Moon /> : <Monitor />,
      run: () =>
        void useApp
          .getState()
          .updateSettings({ theme })
          .catch(() => {}),
    })),
  ];
  if (chat.project) {
    list.splice(3, 0, {
      id: 'terminal',
      label: useTerminal.getState().open ? 'Hide the terminal' : 'Show the terminal',
      group: 'Actions',
      icon: <SquareTerminal />,
      shortcut: 'Ctrl+`',
      run: () => {
        app.go('code');
        useTerminal.getState().toggle();
      },
    });
  }
  return list;
}

function conversations(): Command[] {
  const { lists } = useChat.getState();
  const open = (mode: 'chat' | 'code', id: string) => {
    useApp.getState().go(mode);
    void useChat.getState().open(id);
  };
  return [
    ...lists.chat.slice(0, 30).map((s) => ({
      id: `chat-${s.id}`,
      label: s.title || 'New chat',
      group: 'Chats' as const,
      icon: <MessageSquare />,
      run: () => open('chat', s.id),
    })),
    ...lists.code.slice(0, 30).map((s) => ({
      id: `code-${s.id}`,
      label: s.title || 'New session',
      group: 'Coding sessions' as const,
      icon: <SquareTerminal />,
      run: () => open('code', s.id),
    })),
  ];
}

/** Every word typed appears in the label. */
function matches(label: string, query: string): boolean {
  const text = label.toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((word) => text.includes(word));
}

export function CommandPalette() {
  const open = useApp((s) => s.palette);
  const [query, setQuery] = useState('');
  const [index, setIndex] = useState(0);
  const list = useRef<HTMLDivElement>(null);
  const ids = useId();
  // Built when it opens, so labels reflect the current state.
  const all = useMemo(() => (open ? [...actions(), ...conversations()] : []), [open]);
  const shown = useMemo(() => all.filter((c) => matches(c.label, query)), [all, query]);
  // Consecutive commands of one group, each with its place in `shown`.
  const groups = useMemo(() => {
    const out: { name: string; items: { command: Command; i: number }[] }[] = [];
    shown.forEach((command, i) => {
      const last = out.at(-1);
      if (last?.name === command.group) last.items.push({ command, i });
      else out.push({ name: command.group, items: [{ command, i }] });
    });
    return out;
  }, [shown]);

  useEffect(() => {
    setIndex(0);
  }, [query, open]);
  useEffect(() => {
    if (!open) setQuery('');
  }, [open]);
  useEffect(() => {
    list.current?.querySelector(`[data-index="${index}"]`)?.scrollIntoView({ block: 'nearest' });
  }, [index]);

  const close = () => useApp.getState().setPalette(false);
  const run = (command: Command | undefined) => {
    if (!command) return;
    close();
    command.run();
  };

  // The ARIA combobox pattern: the focus stays in the search box, which points at the option
  // the arrow keys reach (aria-activedescendant), so screen readers announce it.
  const listId = `${ids}-list`;
  const optionId = (i: number) => `${ids}-option-${i}`;
  return (
    <Dialog.Root open={open} onOpenChange={(next) => useApp.getState().setPalette(next)}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content
          aria-describedby={undefined}
          className="fixed left-1/2 top-[14vh] z-50 w-[560px] max-w-[calc(100vw-48px)] -translate-x-1/2 overflow-hidden rounded-card bg-surface shadow-pop ring-1 ring-line outline-none"
        >
          <Dialog.Title className="sr-only">Commands</Dialog.Title>
          <div className="flex items-center gap-2.5 border-b border-line px-4">
            <Search className="size-4 shrink-0 text-text-3" />
            <input
              autoFocus
              role="combobox"
              aria-label="Search commands and conversations"
              aria-autocomplete="list"
              aria-expanded={shown.length > 0}
              aria-controls={shown.length > 0 ? listId : undefined}
              aria-activedescendant={shown[index] ? optionId(index) : undefined}
              placeholder="Type a command or a conversation…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                // Enter and the arrows finish or move through an IME composition: not ours.
                if (e.nativeEvent.isComposing) return;
                if (e.key === 'ArrowDown') {
                  e.preventDefault();
                  setIndex((i) => Math.min(shown.length - 1, i + 1));
                } else if (e.key === 'ArrowUp') {
                  e.preventDefault();
                  setIndex((i) => Math.max(0, i - 1));
                } else if (e.key === 'Enter') {
                  e.preventDefault();
                  run(shown[index]);
                }
              }}
              className="h-12 min-w-0 flex-1 bg-transparent text-[14.5px] outline-none select-text placeholder:text-text-3"
            />
          </div>
          {shown.length === 0 ? (
            <p className="px-3 py-6 text-center text-[13px] text-text-3">Nothing matches.</p>
          ) : (
            <div
              ref={list}
              id={listId}
              role="listbox"
              aria-label="Commands"
              className="max-h-[52vh] overflow-y-auto p-1.5"
            >
              {groups.map((group, n) => (
                <div key={group.name} role="group" aria-labelledby={`${ids}-group-${n}`}>
                  <p
                    id={`${ids}-group-${n}`}
                    className="px-2.5 pb-1 pt-2 text-[11.5px] font-medium text-text-3"
                  >
                    {group.name}
                  </p>
                  {group.items.map(({ command, i }) => (
                    // Not a tab stop: the keyboard reaches options through the search box.
                    <div
                      key={command.id}
                      id={optionId(i)}
                      role="option"
                      aria-selected={i === index}
                      data-index={i}
                      onMouseMove={() => setIndex(i)}
                      onClick={() => run(command)}
                      className={cn(
                        'flex h-9 w-full items-center gap-2.5 rounded-control px-2.5 text-left text-[13.5px] [&_svg]:size-4 [&_svg]:shrink-0 [&_svg]:stroke-[1.75]',
                        i === index ? 'bg-surface-3 text-text' : 'text-text-2',
                      )}
                    >
                      {command.icon}
                      <span className="min-w-0 flex-1 truncate">{command.label}</span>
                      {command.shortcut && (
                        <kbd className="shrink-0 font-sans text-[11.5px] text-text-3">
                          {command.shortcut}
                        </kbd>
                      )}
                    </div>
                  ))}
                </div>
              ))}
            </div>
          )}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
