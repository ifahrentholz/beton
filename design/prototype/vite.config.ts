import path from 'node:path'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// Statischer Prototyp: relative Asset-Pfade + Hash-Routing, damit er ohne Server-Fallback
// (Datei, Artifact, beliebiger statischer Host) funktioniert.
export default defineConfig({
  base: './',
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { '@': path.resolve(__dirname, './src') },
  },
})
