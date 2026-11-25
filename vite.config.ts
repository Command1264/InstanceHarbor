import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
// build-product.ps1 -Development builds with this mode; the sidebar brand image then
// uses the red development icon, like the executable resource chosen by build.rs.
const DEVELOPMENT_CHANNEL = 'development-channel'
export default defineConfig(({ mode }) => ({
  plugins: [vue()],
  resolve: {
    alias: [{
      find: /^@brand-logo(?=\?|$)/,
      replacement: fileURLToPath(new URL(
        mode === DEVELOPMENT_CHANNEL ? './src/assets/instanceharbor-brand-dev.png' : './src/assets/instanceharbor-brand.png',
        import.meta.url,
      )),
    }],
  },
  build: { rollupOptions: { input: { main: 'index.html', console: 'console.html' } } },
}))
