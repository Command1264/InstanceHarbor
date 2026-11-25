// Pure helpers for the test administrator settings; no Vue or i18n imports so Node tests can load it.
export const MAX_TEST_ADMINISTRATORS = 10
export const MAX_PERMISSION_NODES = 50
export const MAX_PERMISSION_NODE_LENGTH = 200
export const LOOKUP_DEBOUNCE_MS = 400

export type PermissionKind = 'all' | 'custom'

/** The same rule the backend applies before it sends any request. */
export function validMinecraftName(name: string) {
  return /^[A-Za-z0-9_]{3,16}$/.test(name)
}

/** The UUID field only ever holds the UUID looked up for this name; Minecraft names ignore case. */
export function uuidBelongsTo(owner: string | undefined, name: string) {
  return owner !== undefined && owner.trim().toLowerCase() === name.trim().toLowerCase()
}

/** One node per line; blank lines are ignored. */
export function parseNodes(text: string) {
  return text.split(/\r?\n/).map(line => line.trim()).filter(Boolean)
}

export function formatNodes(nodes: readonly string[] | null | undefined) {
  return (nodes ?? []).join('\n')
}

export function validNode(node: string) {
  const name = node.startsWith('-') ? node.slice(1) : node
  return name.length >= 1 && name.length <= MAX_PERMISSION_NODE_LENGTH && !name.startsWith('-') && /^[a-z0-9._*-]+$/.test(name)
}

/** The first problem that would make the backend refuse the custom node list. */
export function nodeProblem(text: string): 'empty' | 'invalid' | 'tooMany' | undefined {
  const nodes = [...new Set(parseNodes(text))]
  if (!nodes.length) return 'empty'
  if (!nodes.every(validNode)) return 'invalid'
  if (nodes.length > MAX_PERMISSION_NODES) return 'tooMany'
}

export type TestAdminDraftInput = {
  rows: readonly { name: string; uuid: string }[]
  luckperms: boolean
  op: boolean
  kind: PermissionKind
  nodes: string
  group: string
}

/**
 * The settings payload. Nodes typed while "all" is chosen are kept, not granted.
 * Turning LuckPerms off keeps the nodes and group; only their clear buttons remove them.
 */
export function testAdminPayload(input: TestAdminDraftInput) {
  return {
    administrators: input.rows.map(row => ({ name: row.name.trim(), uuid: row.uuid.trim() })),
    luckperms: input.luckperms,
    op: input.op,
    permissions: input.kind === 'custom' ? parseNodes(input.nodes) : null,
    keptPermissions: input.kind === 'custom' ? [] : parseNodes(input.nodes),
    group: input.group.trim(),
  }
}
