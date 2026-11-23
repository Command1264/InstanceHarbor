import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

const entry = fileURLToPath(new URL('./preview/index.html', import.meta.url))
const wireframe = new URL('./docs/design/wireframe/', import.meta.url)
const references = new Map([
  ['legacy.html', new URL('index.html', wireframe)],
  ['activity-review.html', new URL('activity-review.html', wireframe)],
  ['settings-inventory.html', new URL('settings-inventory.html', wireframe)],
  ['v2-requirements-review.html', new URL('../v2-requirements-review.html', wireframe)],
  ['feature-inventory.json', new URL('feature-inventory.json', wireframe)],
  ['instanceharbor-future-plan.md', new URL('../../instanceharbor-future-plan.md', wireframe)],
  ...['refinement-notes.md', 'design-notes.md', 'validation.json', 'review-notes.md']
    .map(name => [name, new URL(name, wireframe)] as const),
])
const mimeTypes = { html: 'text/html', md: 'text/plain', json: 'application/json' }

export default defineConfig({
  base: './',
  plugins: [
    vue(),
    {
      name: 'offline-preview-comparison',
      configureServer(server) {
        server.middlewares.use((request, response, next) => {
          const name = request.url?.split('?')[0]?.replace(/^\/(?:preview\/)?/, '') ?? ''
          const source = references.get(name)
          if (!source) return next()
          const extension = name.split('.').at(-1) as keyof typeof mimeTypes
          response.setHeader('Content-Type', mimeTypes[extension] + '; charset=utf-8')
          response.end(readFileSync(source))
        })
      },
      generateBundle() {
        for (const [name, source] of references) {
          this.emitFile({ type: 'asset', fileName: 'preview/' + name, source: readFileSync(source) })
          // The preserved legacy HTML uses parent-relative review links.
          if (['v2-requirements-review.html', 'instanceharbor-future-plan.md'].includes(name)) {
            this.emitFile({ type: 'asset', fileName: name, source: readFileSync(source) })
          }
        }
      },
    },
  ],
  build: { outDir: 'dist-preview', cssMinify: false, rollupOptions: { input: entry } },
})
