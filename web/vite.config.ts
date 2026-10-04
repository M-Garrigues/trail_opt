import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import basicSsl from '@vitejs/plugin-basic-ssl';

// /api → handler local (scripts/dev.sh : cargo lambda watch), même origine qu'en prod.
const api = `http://localhost:${process.env.API_PORT ?? 9000}`;

export default defineConfig({
  // LAN=1 (scripts/dev.sh) : HTTPS auto-signé, requis sur téléphone pour géoloc et crypto.subtle
  plugins: [svelte(), ...(process.env.LAN === '1' ? [basicSsl()] : [])],
  // MapLibre 6 charge son worker par new URL(…, import.meta.url) : pas de pré-bundle
  optimizeDeps: { exclude: ['maplibre-gl'] },
  server: {
    port: Number(process.env.WEB_PORT ?? 5173),
    proxy: { '/api': { target: api, rewrite: (p) => '/lambda-url/lambda' + p } },
  },
  build: { target: 'es2022', chunkSizeWarningLimit: 1500 },
  test: { include: ['tests/unit/**/*.test.ts'] },
});
