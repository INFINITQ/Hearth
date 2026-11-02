import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// Tauri needs a fixed dev port and no screen clearing, so its own output stays visible.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: { host: 'localhost', port: 5180, strictPort: true },
  build: { target: 'es2023', outDir: 'dist', emptyOutDir: true, sourcemap: false },
});
