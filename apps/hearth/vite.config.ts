import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { type Plugin, defineConfig } from 'vite';

/**
 * xterm.js gives a namespace object its own `toString` by plain assignment
 * (`KeyCodeUtils.toString = …`). Tauri's `freezePrototype` makes `Object.prototype.toString`
 * read-only, so that assignment throws. Defining the property does the same thing and works.
 */
function xtermWithFrozenPrototype(): Plugin {
  return {
    name: 'hearth:xterm-frozen-prototype',
    enforce: 'pre',
    transform(code, id) {
      if (!/@xterm[\\/]xterm[\\/]lib[\\/]xterm\.m?js/.test(id)) return null;
      const fixed = code.replace(
        /\b([A-Za-z_$][\w$]*)\.toString=([A-Za-z_$][\w$]*);/,
        'Object.defineProperty($1,"toString",{value:$2,writable:!0,enumerable:!0,configurable:!0});',
      );
      if (fixed === code) this.error('xterm changed: its toString assignment was not found');
      return { code: fixed, map: null };
    },
  };
}

// Tauri needs a fixed dev port and no screen clearing, so its own output stays visible.
export default defineConfig({
  plugins: [xtermWithFrozenPrototype(), react(), tailwindcss()],
  // Served as is (not pre-bundled) in development, so the fix above applies there too.
  optimizeDeps: { exclude: ['@xterm/xterm'] },
  clearScreen: false,
  server: { host: 'localhost', port: 5180, strictPort: true },
  build: { target: 'es2023', outDir: 'dist', emptyOutDir: true, sourcemap: false },
});
