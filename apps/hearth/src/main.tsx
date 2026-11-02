import '@fontsource-variable/inter';
import '@fontsource-variable/jetbrains-mono';
import './styles/app.css';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { backend } from './lib/backend';
import { applyTheme, resolveTheme } from './lib/theme';

// Theme before the first render, so the first frame already has the right colors.
const boot = backend().boot();
applyTheme(resolveTheme(boot.settings.theme, boot.systemDark));

const root = document.getElementById('root');
if (!root) throw new Error('index.html has no #root element');
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

// The window starts hidden; show it once the first frame has painted, so it never flashes.
requestAnimationFrame(() =>
  requestAnimationFrame(() => {
    backend()
      .appReady()
      .catch((err: unknown) => console.error('app_ready failed', err));
  }),
);
