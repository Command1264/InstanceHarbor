export type PolicyProfile = { family: string; version: string }

export type PolicyRow = {
  key: string
  label: string
  category: string
  source: string
  value: string | null
  inherited?: string | null
  kind: 'integer' | 'text' | 'boolean' | 'choice'
  min: number | null
  max: number | null
  choices: string[]
  status: 'available' | 'unsupported' | 'unverified'
  evidence: 'unverified' | 'unsupported'
  reason: string
  editable?: boolean
  file?: string
}

export type InitializationSnapshot = {
  revision: string
  profile: PolicyProfile
  profiles: PolicyProfile[]
  rows: PolicyRow[]
}

export type WorldSnapshot = {
  revision: string
  profile: PolicyProfile | null
  rows: PolicyRow[]
  pending?: { state: 'Queued' | 'Applying' | 'Conflict'; values: Record<string, string> } | null
  yamlPreflight?: {
    state: 'Compared' | 'Unverified'
    source?: 'StoppedReadback' | 'Launch'
    changes: { key: string; file: string; before: string | null; after: string | null }[]
    filesChanged?: string[]
  }
  queued?: boolean
  applied?: boolean
}

export type WorldPreview = {
  revision: string
  preview: string
  changes: { key: string; before: string; after: string; path?: string }[]
}

export type WorldLiveValue = { value?: string | null; target?: string | null; state: string }

const worldRuleSaveStates = ['Queued', 'AwaitingNormalStop', 'Unconfirmed', 'Saved'] as const
export type WorldRuleSaveState = typeof worldRuleSaveStates[number]
export type WorldRuleSaveReceipt = {
  name: string
  request: string
  count: number
  rows: Record<string, { target: string; state: WorldRuleSaveState }>
}

export function verifiedWorldRuleReceipt(receipt: WorldRuleSaveReceipt, name: string, request: string, rules: Record<string, string>) {
  const requested = Object.entries(rules)
  return receipt?.name === name && receipt.request === request && receipt.count === requested.length
    && !!receipt.rows && Object.keys(receipt.rows).length === requested.length
    && requested.every(([key, target]) => {
      const result = receipt.rows[key]
      return result?.target === target && worldRuleSaveStates.includes(result.state)
    })
}

export function isWorldRule(row: PolicyRow) {
  return row.category === '遊戲規則' && row.key !== 'initialTime'
}

export function validWorldRuleValue(row: PolicyRow, value: string) {
  if (row.kind === 'boolean') return value === 'true' || value === 'false'
  if (row.kind !== 'integer') return false
  const number = Number(value)
  return /^\d+$/.test(value) && Number.isSafeInteger(number)
    && number >= (row.min ?? 0) && number <= (row.max ?? 32767)
}
