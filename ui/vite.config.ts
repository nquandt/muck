import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// base: './' so asset URLs resolve correctly when served from an arbitrary path by the
// embedded Rust static-file handler (src/bin/local.rs), not just from a fixed site root.
export default defineConfig({
  base: './',
  plugins: [react()],
})
