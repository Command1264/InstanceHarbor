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

/** Look a name up only while the UUID is empty or was filled in by an earlier lookup. */
export function shouldLookup(row: { name: string; uuid: string }, autoFilled: boolean) {
  return validMinecraftName(row.name.trim()) && (row.uuid.trim() === '' || autoFilled)
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
 * The settings payload. Custom nodes are sent only when that choice is made.
 * Turning LuckPerms off keeps the nodes and group; only {@link clearedLuckPerms} removes them.
 */
export function testAdminPayload(input: TestAdminDraftInput) {
  return {
    administrators: input.rows.map(row => ({ name: row.name.trim(), uuid: row.uuid.trim() })),
    luckperms: input.luckperms,
    op: input.op,
    permissions: input.kind === 'custom' ? parseNodes(input.nodes) : null,
    group: input.group.trim(),
  }
}

/** What the clear button resets the LuckPerms grant content to. */
export function clearedLuckPerms(): Pick<TestAdminDraftInput, 'kind' | 'nodes' | 'group'> {
  return { kind: 'all', nodes: '', group: '' }
}
