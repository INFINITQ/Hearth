import type { ThemeSetting } from './types';

export type ResolvedTheme = 'light' | 'dark';

/** The theme to show: an explicit choice wins, "system" follows Windows. */
export function resolveTheme(setting: ThemeSetting, systemDark: boolean): ResolvedTheme {
  if (setting === 'system') return systemDark ? 'dark' : 'light';
  return setting;
}

/** Switches the document's design tokens (styles/app.css keys them on `data-theme`). */
export function applyTheme(theme: ResolvedTheme) {
  const root = document.documentElement;
  root.dataset.theme = theme;
  root.style.colorScheme = theme;
}

/** Calls `listener` whenever Windows switches between light and dark. */
export function watchSystemTheme(listener: (dark: boolean) => void): () => void {
  const query = matchMedia('(prefers-color-scheme: dark)');
  const onChange = (event: MediaQueryListEvent) => listener(event.matches);
  query.addEventListener('change', onChange);
  return () => query.removeEventListener('change', onChange);
}
