import { PanelLeft, SquarePen } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { type WindowControls, backend } from '../lib/backend';
import { useApp } from '../stores/app';
import { useChat } from '../stores/chat';
import { ModelPicker } from './ModelPicker';
import { newConversation } from './Sidebar';
import { IconButton } from './ui/Button';

/** The top of the main area: conversation title, model picker and the window's caption buttons. */
export function TitleBar() {
  const collapsed = useApp((s) => s.settings.sidebarCollapsed);
  const mode = useApp((s) => s.mode);
  const screen = useApp((s) => s.screen);
  const session = useChat((s) => s.session);
  const inConversation = screen === 'chat' || screen === 'code';
  const title = inConversation && session?.mode === screen ? session.title : '';
  const focused = useWindowFocus();
  return (
    <header
      className="titlebar flex h-10 shrink-0 items-center"
      data-focused={focused}
      data-tauri-drag-region
    >
      {collapsed && (
        <div className="flex items-center gap-0.5 pl-2">
          <IconButton
            label="Show sidebar"
            shortcut="Ctrl+B"
            onClick={() => useApp.getState().toggleSidebar()}
          >
            <PanelLeft />
          </IconButton>
          <IconButton
            label={mode === 'code' ? 'New session' : 'New chat'}
            shortcut="Ctrl+N"
            onClick={() => newConversation(mode)}
          >
            <SquarePen />
          </IconButton>
        </div>
      )}
      <div
        className="min-w-0 flex-1 truncate px-4 text-[13px] font-medium text-text-2"
        data-tauri-drag-region
      >
        {title}
      </div>
      {inConversation && <ModelPicker />}
      <CaptionButtons controls={backend().window} />
    </header>
  );
}

function useWindowFocus(): boolean {
  const [focused, setFocused] = useState(() => document.hasFocus());
  useEffect(() => {
    const on = () => setFocused(true);
    const off = () => setFocused(false);
    window.addEventListener('focus', on);
    window.addEventListener('blur', off);
    return () => {
      window.removeEventListener('focus', on);
      window.removeEventListener('blur', off);
    };
  }, []);
  return focused;
}

// Segoe Fluent Icons code points, as Windows 11's own caption buttons use.
const GLYPH = {
  minimize: '',
  maximize: '',
  restore: '',
  close: '',
} as const;

/** Minimize, maximize/restore (with Windows 11 Snap Layouts) and close. */
function CaptionButtons({ controls }: { controls: WindowControls }) {
  const maximize = useRef<HTMLButtonElement>(null);
  const [maximized, setMaximized] = useState(false);
  const [state, setState] = useState<'idle' | 'hover' | 'pressed'>('idle');

  useEffect(() => {
    const report = () => {
      const box = maximize.current?.getBoundingClientRect();
      if (!box) return;
      controls.reportMaximizeRect({
        right: window.innerWidth - box.right,
        top: box.top,
        width: box.width,
        height: box.height,
      });
      void controls.isMaximized().then(setMaximized);
    };
    report();
    const observer = new ResizeObserver(report);
    observer.observe(document.documentElement);
    const stopResize = controls.onResized(report);
    const stopCaption = controls.onCaption((caption) => {
      setState(caption === 'press' ? 'pressed' : caption === 'leave' ? 'idle' : 'hover');
    });
    return () => {
      observer.disconnect();
      stopResize();
      stopCaption();
    };
  }, [controls]);

  return (
    <div className="caption">
      <button
        type="button"
        aria-label="Minimize"
        title="Minimize"
        onClick={() => controls.minimize()}
      >
        {GLYPH.minimize}
      </button>
      <button
        type="button"
        ref={maximize}
        aria-label={maximized ? 'Restore' : 'Maximize'}
        data-state={state}
        onClick={() => controls.toggleMaximize()}
      >
        {maximized ? GLYPH.restore : GLYPH.maximize}
      </button>
      <button
        type="button"
        className="caption-close"
        aria-label="Close"
        title="Close"
        onClick={() => controls.close()}
      >
        {GLYPH.close}
      </button>
    </div>
  );
}
