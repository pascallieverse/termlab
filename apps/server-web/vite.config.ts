import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath, URL } from 'node:url';
import { defineConfig } from 'vite';

// The web UI of termlab-server. The server embeds `dist/` and serves it; in
// development, `npm run dev` proxies the API (and its websockets) to a
// server running on 127.0.0.1:8484.
export default defineConfig({
    plugins: [react(), tailwindcss()],
    resolve: {
        alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
    },
    build: {
        outDir: 'dist',
        emptyOutDir: true,
        sourcemap: false,
        rollupOptions: {
            output: {
                manualChunks: { xterm: ['@xterm/xterm', '@xterm/addon-fit'] },
            },
        },
    },
    server: {
        port: 5174,
        proxy: {
            '/api': {
                target: 'http://127.0.0.1:8484',
                changeOrigin: false,
                ws: true,
            },
        },
    },
});
