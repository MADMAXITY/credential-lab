import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  // Tauri expects a fixed port
  server: {
    port: 5173,
    strictPort: true,
  },
  // Prevent vite from obscuring Rust errors
  clearScreen: false,
})
