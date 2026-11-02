import {
  BookOpen,
  Boxes,
  MessageSquare,
  PanelLeft,
  Search,
  Settings,
  SquarePen,
  SquareTerminal,
  X,
} from 'lucide-react';
import { type ReactNode, useEffect, useState } from 'react';
import { cn } from '../lib/cn';
import { type Mode, useApp } from '../stores/app';
import { useChat } from '../stores/chat';
import { HearthMark } from './HearthMark';
import { SessionList } from './SessionList';
import { ProjectSwitcher } from './code/ProjectSwitcher';
import { IconButton } from './ui/Button';
import { Segmented } from './ui/Segmented';

/** Starts a new conversation in the current mode. */
export function newConversation(mode: Mode) {
  useApp.getState().setMode(mode);
  useChat.getState().newChat();
}

/** The left column: brand, Chat/Code switch, new conversation, search, history, app pages. */
export function Sidebar() {
  const mode = useApp((s) => s.mode);
  const screen = useApp((s) => s.screen);
  const app = useApp.getState;

  return (
    <aside aria-label="Sidebar" className="flex w-[264px] shrink-0 flex-col bg-sidebar">
      <div className="flex h-10 items-center gap-2 pl-4 pr-2" data-tauri-drag-region>
        <HearthMark className="size-[18px] text-accent" />
        <span className="text-[13.5px] font-semibold tracking-[-0.01em]" data-tauri-drag-region>
          Hearth
        </span>
        <div className="flex-1" data-tauri-drag-region />
        <IconButton label="Hide sidebar" shortcut="Ctrl+B" onClick={() => app().toggleSidebar()}>
          <PanelLeft />
        </IconButton>
      </div>

      <div className="space-y-2 px-3 pb-2 pt-1.5">
        <Segmented<Mode>
          label="Mode"
          value={mode}
          onChange={(next) => {
            app().setMode(next);
            void useChat.getState().search('', next);
          }}
          options={[
            { value: 'chat', label: 'Chat', icon: <MessageSquare /> },
            { value: 'code', label: 'Code', icon: <SquareTerminal /> },
          ]}
        />
        <button
          type="button"
          onClick={() => newConversation(mode)}
          className="flex h-9 w-full items-center gap-2.5 rounded-control px-2.5 text-[13.5px] font-medium text-text transition-colors duration-150 hover:bg-surface-3/70 [&_svg]:size-4 [&_svg]:stroke-[1.75]"
        >
          <SquarePen className="text-accent" />
          {mode === 'code' ? 'New session' : 'New chat'}
          <kbd className="ml-auto font-sans text-[11px] font-normal text-text-3">Ctrl N</kbd>
        </button>
        {mode === 'code' && <ProjectSwitcher />}
        <SearchBox mode={mode} />
      </div>

      <nav
        aria-label={mode === 'code' ? 'Sessions' : 'Conversations'}
        className="min-h-0 flex-1 overflow-y-auto px-2"
      >
        <SessionList mode={mode} />
      </nav>

      <div className="space-y-0.5 px-2 pb-2.5 pt-2">
        <NavItem
          icon={<BookOpen />}
          label="Guide"
          active={screen === 'guide'}
          onClick={() => app().go('guide')}
        />
        <NavItem
          icon={<Boxes />}
          label="Models"
          active={screen === 'models'}
          onClick={() => app().go('models')}
        />
        <NavItem
          icon={<Settings />}
          label="Settings"
          active={screen === 'settings'}
          onClick={() => app().go('settings')}
        />
      </div>
    </aside>
  );
}

function SearchBox({ mode }: { mode: Mode }) {
  const query = useChat((s) => s.query);
  const [text, setText] = useState(query);
  useEffect(() => setText(query), [query]);
  useEffect(() => {
    const timer = setTimeout(() => {
      if (text !== useChat.getState().query) void useChat.getState().search(text, mode);
    }, 180);
    return () => clearTimeout(timer);
  }, [text, mode]);
  return (
    <label className="flex h-8 items-center gap-2 rounded-control px-2.5 text-[13px] text-text-3 ring-line transition-colors focus-within:bg-surface focus-within:ring-1 hover:bg-surface-3/50 [&_svg]:size-[15px]">
      <Search className="shrink-0" />
      <input
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => e.key === 'Escape' && setText('')}
        placeholder="Search"
        aria-label="Search conversations"
        className="min-w-0 flex-1 bg-transparent text-text outline-none select-text placeholder:text-text-3"
      />
      {text && (
        <button
          type="button"
          aria-label="Clear search"
          onClick={() => setText('')}
          className="text-text-3 hover:text-text"
        >
          <X className="size-3.5" />
        </button>
      )}
    </label>
  );
}

function NavItem({
  icon,
  label,
  active,
  onClick,
}: {
  icon: ReactNode;
  label: string;
  active: boolean;
  onClick(): void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={active ? 'page' : undefined}
      className={cn(
        'flex h-8 w-full items-center gap-2.5 rounded-control px-2.5 text-[13px] transition-colors duration-150 [&_svg]:size-4 [&_svg]:stroke-[1.75]',
        active
          ? 'bg-surface-3 font-medium text-text'
          : 'text-text-2 hover:bg-surface-3/70 hover:text-text',
      )}
    >
      {icon}
      {label}
    </button>
  );
}
