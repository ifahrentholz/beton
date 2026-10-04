import path from 'node:path'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// Ausgeliefert von `beton serve` (WEB-001): absolute Pfade unter `/`, alle Assets im Bundle,
// keine fremden Origins (ADR-0033).
export default defineConfig({
  base: '/',
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': path.resolve(import.meta.dirname, './src'),
      '@beton/sdk': path.resolve(import.meta.dirname, '../../packages/sdk-ts/src/index.ts'),
    },
  },
  build: {
    target: 'es2022',
    sourcemap: false,
    // Monaco (WEB-009) ist ein großer, nur bei Bedarf geladener Chunk.
    chunkSizeWarningLimit: 4500,
  },
  server: {
    proxy: {
      '/v1': { target: 'http://127.0.0.1:7420', ws: true },
      '/auth': 'http://127.0.0.1:7420',
    },
  },
})
