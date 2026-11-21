import { reactive } from 'vue'
import { manage, rawError } from './manager'

export type CatalogFamily = 'PAPER' | 'FOLIA'
export type CatalogBuild = {
  family: CatalogFamily
  minecraft: string
  build: string
  sha256: string
}
export type CatalogEntry<T> = {
  value?: T
  pending: boolean
  manual: boolean
  failed: boolean
}

const families = ['PAPER', 'FOLIA'] as const
const storageKey = 'instanceharbor.v2.core-catalog.v1'
const maximumVersions = 256
const maximumBuilds = 32
const maximumStoredLength = 64 * 1024
export const catalogRefreshInterval = 30 * 60 * 1000

function versionList(input: unknown): string[] {
  if (!Array.isArray(input) || input.length > maximumVersions
    || input.some(version => typeof version !== 'string' || !validVersion(version))) {
    throw new Error('InvalidCoreCatalogVersions')
  }
  return [...new Set<string>(input)]
}

function validVersion(input: string) {
  return input.length <= 32 && /^\d+(\.\d+)+$/.test(input)
}

function validBuild(input: unknown, family: CatalogFamily, minecraft: string): CatalogBuild {
  const build = input as Partial<CatalogBuild> | null
  if (build?.family !== family || build.minecraft !== minecraft
    || typeof build.build !== 'string' || !/^[1-9]\d{0,19}$/.test(build.build)
    || typeof build.sha256 !== 'string' || !/^[a-f0-9]{64}$/.test(build.sha256)) {
    throw new Error('InvalidCoreCatalogBuild')
  }
  return { family, minecraft, build: build.build, sha256: build.sha256 }
}

function entry<T>(value?: T): CatalogEntry<T> {
  return { value, pending: false, manual: false, failed: false }
}

const buildKey = (family: CatalogFamily, minecraft: string) => `${family}/${minecraft}`

export function useCoreCatalog() {
  const versions = reactive<Record<CatalogFamily, CatalogEntry<string[]>>>({
    PAPER: entry(), FOLIA: entry(),
  })
  const builds = reactive<Record<string, CatalogEntry<CatalogBuild | null>>>({})
  const order = new Set<string>()

  function buildEntry(family: CatalogFamily, minecraft: string) {
    const key = buildKey(family, minecraft)
    builds[key] ??= entry()
    return builds[key]
  }

  function remember(key: string) {
    order.delete(key)
    order.add(key)
    while (order.size > maximumBuilds) {
      const oldest = order.values().next().value!
      order.delete(oldest)
      if (!builds[oldest].pending) delete builds[oldest]
    }
  }

  function restore() {
    try {
      const raw = localStorage.getItem(storageKey)
      if (!raw || raw.length > maximumStoredLength) return
      const saved = JSON.parse(raw)
      for (const family of families) {
        try {
          if (saved.versions?.[family] !== undefined) versions[family].value = versionList(saved.versions[family])
        } catch { /* A malformed snapshot can be replaced by the next network reply. */ }
      }
      for (const cached of Array.isArray(saved.builds) ? saved.builds.slice(-maximumBuilds) : []) {
        const { family, minecraft, value } = cached ?? {}
        if ((family !== 'PAPER' && family !== 'FOLIA') || typeof minecraft !== 'string' || !validVersion(minecraft)) continue
        try {
          buildEntry(family, minecraft).value = value === null ? null : validBuild(value, family, minecraft)
          remember(buildKey(family, minecraft))
        } catch { /* Ignore invalid or mismatched cache entries independently. */ }
      }
    } catch { /* Storage can be unavailable. In-memory refresh still works. */ }
  }

  function save() {
    try {
      const stored = JSON.stringify({
        versions: { PAPER: versions.PAPER.value, FOLIA: versions.FOLIA.value },
        builds: [...order].map(key => {
          const [family, minecraft] = key.split('/')
          return { family, minecraft, value: builds[key].value }
        }),
      })
      localStorage.setItem(storageKey, stored)
    } catch { /* A storage error does not invalidate a network result. */ }
  }

  async function refresh<T>(
    target: CatalogEntry<T>, manual: boolean, load: () => Promise<T>, onSuccess = save,
  ) {
    if (manual) target.manual = true
    if (target.pending) return
    target.pending = true
    try {
      const value = await load()
      if (JSON.stringify(target.value) !== JSON.stringify(value)) target.value = value
      target.failed = false
      onSuccess()
    } catch {
      target.failed = target.value === undefined || target.manual
    } finally {
      target.pending = false
      target.manual = false
    }
  }

  function refreshVersions(family: CatalogFamily, manual = false) {
    return refresh(versions[family], manual, async () => {
      const reply = await manage<{ versions: unknown }>(['core-catalog-versions', family], manual ? 'manual' : 'background')
      return versionList(reply.versions)
    })
  }

  function refreshBuild(family: CatalogFamily, minecraft: string, manual = false) {
    if (!validVersion(minecraft)) return
    const key = buildKey(family, minecraft)
    const target = buildEntry(family, minecraft)
    return refresh(target, manual, async () => {
      try {
        const reply = await manage<unknown>(['core-catalog-build', family, minecraft], manual ? 'manual' : 'background')
        return validBuild(reply, family, minecraft)
      } catch (error) {
        if (['CoreNoStableBuild', 'CreationNoStableBuild'].includes(rawError(error))) return null
        throw error
      }
    }, () => { remember(key); save() })
  }

  restore()
  return { versions, buildEntry, refreshVersions, refreshBuild }
}
