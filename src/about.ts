// What the About section shows; the backend reports it (about_links.rs).
import { tr } from './i18n.ts'

/** Fixed link names the backend opens. The page never sends a URL or path. */
export type AboutLink = 'repository' | 'issues' | 'license' | 'notices'
export interface AboutInfo {
  version: string
  /** Seven hex digits, `-dirty` when built with uncommitted changes, or `unknown` without Git. */
  commit: string
  /** `InstanceHarbor` or `InstanceHarbor Dev`. */
  channel: string
  /** The development workspace folder name; null in the stable channel. */
  workspace: string | null
  author: string
  repository: string
  issues: string
  /** Whether LICENSE and THIRD-PARTY-NOTICES.txt are beside the executable. */
  license: boolean
  notices: boolean
}

export function aboutCommitText(commit: string, t: typeof tr = tr) {
  if (commit === 'unknown') return t('components.about.commitUnknown')
  return commit.endsWith('-dirty') ? t('components.about.commitDirty', { commit }) : commit
}

/** The product name the rail shows; matches `desktop_channel::product_title`, for example `InstanceHarbor Dev — accept-52b`. */
export function productTitle(info?: Pick<AboutInfo, 'channel' | 'workspace'>) {
  if (!info) return 'InstanceHarbor'
  return info.workspace ? `${info.channel} \u2014 ${info.workspace}` : info.channel
}
