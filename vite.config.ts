import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
// packaging/build-product.ps1 -Development builds with this mode; the sidebar brand image then
// uses the red development icon, like the executable resource chosen by build.rs.
const DEVELOPMENT_CHANNEL = 'development-channel'
// src-tauri/icons/channels.json pairs each channel with its icons; nothing else names them (#116).
const channelIcons = JSON.parse(readFileSync(new URL('./src-tauri/icons/channels.json', import.meta.url), 'utf8'))

/** The `@brand-logo` import: the sidebar image of the channel that this Vite mode builds. */
export function brandLogoAlias(mode: string) {
  const channel = mode === DEVELOPMENT_CHANNEL ? 'development' : 'product'
  return {
    find: /^@brand-logo(?=\?|$)/,
    replacement: fileURLToPath(new URL(`./${channelIcons[channel].brand}`, import.meta.url)),
  }
}

export default defineConfig(({ mode }) => ({
  plugins: [vue()],
  resolve: { alias: [brandLogoAlias(mode)] },
  build: { rollupOptions: { input: { main: 'index.html', console: 'console.html' } } },
}))
