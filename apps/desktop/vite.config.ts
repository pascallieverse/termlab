import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed dev port and needs to see Rust compile errors in the terminal.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  // A local desktop bundle: chunk size only affects startup from disk.
  build: { target: "safari16", outDir: "dist", emptyOutDir: true, chunkSizeWarningLimit: 1500 },
});
