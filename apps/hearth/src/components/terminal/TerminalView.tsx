// One shell in the terminal panel, drawn by xterm.js. The shell runs in Rust; this view attaches
// to it (getting what it showed so far), types into it and keeps its size in step with the panel.

import { FitAddon } from '@xterm/addon-fit';
import { type ITheme, Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { useEffect, useRef } from 'react';
import { backend, errorMessage } from '../../lib/backend';
import type { TerminalInfo } from '../../lib/types';
import { useTerminal } from '../../stores/terminal';

const FONT = "'JetBrains Mono Variable', 'Cascadia Mono', Consolas, monospace";

const LIGHT: ITheme = {
  background: '#ffffff',
  foreground: '#1f1d1a',
  cursor: '#d97706',
  cursorAccent: '#ffffff',
  selectionBackground: 'rgba(217, 119, 6, 0.22)',
  black: '#1f1d1a',
  red: '#c0362c',
  green: '#2f7d32',
  yellow: '#9a6700',
  blue: '#2563eb',
  magenta: '#9333ea',
  cyan: '#0e7c86',
  white: '#6b655e',
  brightBlack: '#9a938b',
  brightRed: '#dc2626',
  brightGreen: '#15803d',
  brightYellow: '#b45309',
  brightBlue: '#1d4ed8',
  brightMagenta: '#7e22ce',
  brightCyan: '#0f766e',
  brightWhite: '#3a3631',
};

const DARK: ITheme = {
  background: '#21201d',
  foreground: '#ece9e4',
  cursor: '#f59e0b',
  cursorAccent: '#21201d',
  selectionBackground: 'rgba(245, 158, 11, 0.3)',
  black: '#302e2a',
  red: '#f87171',
  green: '#86c27a',
  yellow: '#e5b454',
  blue: '#7aa7f0',
  magenta: '#c792ea',
  cyan: '#6fc3c9',
  white: '#d4cfc8',
  brightBlack: '#78726b',
  brightRed: '#fca5a5',
  brightGreen: '#a7d99c',
  brightYellow: '#f2cb77',
  brightBlue: '#9dc0f7',
  brightMagenta: '#ddb3f5',
  brightCyan: '#93d8dd',
  brightWhite: '#ffffff',
};

const themeNow = () => (document.documentElement.dataset.theme === 'dark' ? DARK : LIGHT);

/** Measuring the font before it has loaded would size every cell wrong. */
const fontReady = document.fonts.load("13px 'JetBrains Mono Variable'").catch(() => []);

export default function TerminalView({ info, active }: { info: TerminalInfo; active: boolean }) {
  const box = useRef<HTMLDivElement>(null);
  const view = useRef<Terminal | null>(null);

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    let disposed = false;
    const cleanup: Array<() => void> = [];

    void fontReady.then(() => {
      if (disposed) return;
      const terminal = new Terminal({
        fontFamily: FONT,
        fontSize: 13,
        lineHeight: 1.2,
        cursorBlink: true,
        scrollback: 5000,
        theme: themeNow(),
      });
      const fit = new FitAddon();
      terminal.loadAddon(fit);
      terminal.attachCustomKeyEventHandler((event) => {
        if (event.type !== 'keydown' || !event.ctrlKey) return true;
        // Ctrl+` belongs to the app (it hides the panel).
        if (event.code === 'Backquote') return false;
        const key = event.key.toLowerCase();
        // Ctrl+C copies a selection (and interrupts otherwise); Ctrl+V pastes.
        if (key === 'c' && terminal.hasSelection()) {
          void navigator.clipboard.writeText(terminal.getSelection());
          terminal.clearSelection();
          return false;
        }
        if (key === 'v') return false;
        return true;
      });
      terminal.open(el);
      view.current = terminal;

      const typed = terminal.onData((data) => {
        void backend()
          .terminalWrite(info.id, data)
          .catch(() => {});
      });
      const resized = terminal.onResize(({ cols, rows }) => {
        void backend()
          .terminalResize(info.id, cols, rows)
          .catch(() => {});
      });
      const refit = () => {
        if (disposed || el.clientWidth === 0 || el.clientHeight === 0) return;
        fit.fit();
      };
      const sizes = new ResizeObserver(() => requestAnimationFrame(refit));
      sizes.observe(el);
      refit();
      const themes = new MutationObserver(() => {
        terminal.options.theme = themeNow();
      });
      themes.observe(document.documentElement, {
        attributes: true,
        attributeFilter: ['data-theme'],
      });
      void backend()
        .terminalAttach(info.id, (event) => {
          if (disposed) return;
          if (event.type === 'output') {
            terminal.write(event.data);
          } else {
            const code = event.code == null ? '' : ` with code ${event.code}`;
            terminal.write(`\r\n\x1b[2m[The shell ended${code}.]\x1b[0m\r\n`);
            useTerminal.getState().exited(info.id, event.code);
          }
        })
        .catch((err: unknown) => terminal.write(`\x1b[31m${errorMessage(err)}\x1b[0m\r\n`));
      if (active) terminal.focus();

      cleanup.push(() => {
        sizes.disconnect();
        themes.disconnect();
        typed.dispose();
        resized.dispose();
        terminal.dispose();
        view.current = null;
      });
    });

    return () => {
      disposed = true;
      cleanup.forEach((dispose) => dispose());
    };
    // The terminal is made once per shell; `active` only moves the focus (below).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [info.id]);

  useEffect(() => {
    if (active) view.current?.focus();
  }, [active]);

  return <div ref={box} className="h-full w-full" data-terminal={info.id} />;
}
