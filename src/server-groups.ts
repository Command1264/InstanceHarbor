// Server groups (#37): one group per instance, like a folder. The data comes from
// `server-groups` (workspace file `server-groups.json`); this file only arranges rows.

export interface ServerGroup { id: string; name: string; collapsed?: boolean }
export interface ServerGroups {
  groups: ServerGroup[]
  /** Instance name → group id. Missing names are ungrouped. */
  members: Record<string, string>
  ungroupedCollapsed?: boolean
  revision?: string
}
export type ServerGroupEdit =
  | { op: 'create'; name: string }
  | { op: 'rename'; id: string; name: string }
  | { op: 'delete'; id: string }
  | { op: 'assign'; id: string | null; names: string[] }
  | { op: 'collapse'; id: string | null; collapsed: boolean }

export const emptyServerGroups: ServerGroups = { groups: [], members: {} }

/** The filter values: every group, only ungrouped instances, or one group by id. */
export type GroupFilter = 'all' | 'ungrouped' | `group:${string}`

/** The id of the group an instance is in, or `null` for "Ungrouped". */
export function groupOf(data: ServerGroups, name: string): string | null {
  const id = Object.hasOwn(data.members, name) ? data.members[name] : undefined
  return id && data.groups.some(group => group.id === id) ? id : null
}

export function matchesGroup(data: ServerGroups, name: string, filter: GroupFilter): boolean {
  if (filter === 'all') return true
  const id = groupOf(data, name)
  return filter === 'ungrouped' ? id === null : id === filter.slice('group:'.length)
}

/** A filter that names a group that no longer exists falls back to every group. */
export function validFilter(data: ServerGroups, filter: GroupFilter): GroupFilter {
  if (filter === 'all' || filter === 'ungrouped') return filter
  const id = filter.slice('group:'.length)
  return data.groups.some(group => group.id === id) ? filter : 'all'
}

export interface ListSection<Row> {
  /** `null` is "Ungrouped". */
  id: string | null
  name: string | null
  /** Rows shown in this section, in list order. Pinned rows are shown above the groups instead. */
  rows: Row[]
  /** The whole group, whatever the search shows; batch actions use these rows. */
  members: Row[]
  running: number
  collapsed: boolean
}

export type ListEntry<Row> = { kind: 'row'; row: Row } | { kind: 'group'; section: ListSection<Row> }

/**
 * Arranges sorted, filtered rows. Pinned rows stay on top; the other rows follow
 * in their groups, in the groups' order, then "Ungrouped". Without groups the
 * list stays flat, as before #37.
 *
 * `all` holds every row so a group header can count and act on the whole group.
 * `showEmpty` lists groups with no visible rows (when nothing filters the list).
 */
export function listEntries<Row extends { name: string }>(options: {
  visible: readonly Row[]
  all: readonly Row[]
  pins: readonly string[]
  data: ServerGroups
  running: (row: Row) => boolean
  showEmpty: boolean
}): ListEntry<Row>[] {
  const { visible, all, pins, data, running, showEmpty } = options
  if (!data.groups.length) return visible.map(row => ({ kind: 'row', row }))
  const pinned = new Set(pins)
  const entries: ListEntry<Row>[] = visible.filter(row => pinned.has(row.name)).map(row => ({ kind: 'row', row }))
  const sections: { id: string | null; name: string | null; collapsed: boolean }[] = [
    ...data.groups.map(group => ({ id: group.id, name: group.name, collapsed: !!group.collapsed })),
    { id: null, name: null, collapsed: !!data.ungroupedCollapsed },
  ]
  for (const section of sections) {
    const members = all.filter(row => groupOf(data, row.name) === section.id)
    const rows = visible.filter(row => !pinned.has(row.name) && groupOf(data, row.name) === section.id)
    const shownInGroup = visible.some(row => groupOf(data, row.name) === section.id)
    // "Ungrouped" appears only when it has rows; named groups also appear empty.
    if (!shownInGroup && !(showEmpty && section.id !== null)) continue
    entries.push({ kind: 'group', section: { ...section, rows, members, running: members.filter(running).length } })
    if (!section.collapsed) for (const row of rows) entries.push({ kind: 'row', row })
  }
  return entries
}

/** The order rows appear in, for Shift range selection; rows of collapsed groups are skipped. */
export function shownRows<Row>(entries: readonly ListEntry<Row>[]): Row[] {
  return entries.flatMap(entry => entry.kind === 'row' ? [entry.row] : [])
}
