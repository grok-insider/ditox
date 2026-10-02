import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  server: { port: 1420, strictPort: true },
  // assetsInlineLimit 0: the Tauri CSP (tauri.conf.json) has no `font-src data:`, and Vite would otherwise inline the two
  // font files smaller than 4 kB (IBM Plex Sans Condensed cyrillic-ext in @fabrials/ui >= 0.7) as data: URLs the CSP blocks.
  build: { outDir: "dist", emptyOutDir: true, assetsInlineLimit: 0 },
});
