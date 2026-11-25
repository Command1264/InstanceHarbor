import type { Instance } from './manager.ts'

export interface StateFlags { queued: boolean; starting: boolean; stopping: boolean }

// While a stop is in progress the server closes its port before Java and the
// Host exit, so the observed state passes through Starting and RunUnconfirmed.
// A stop sent from InstanceHarbor or typed in the console shows Stopping instead.
const stoppable = ['Listening', 'Starting', 'RunUnconfirmed']

export function presentedState(row: Pick<Instance, 'state' | 'stopPhase'>, flags: StateFlags): string {
  if (row.state === 'Stopped') return flags.queued ? 'Queued' : flags.starting ? 'Starting' : 'Stopped'
  if ((flags.stopping || !!row.stopPhase) && stoppable.includes(row.state)) return 'Stopping'
  return row.state
}

/**
 * Every state the instance list can show, and the status colour it uses.
 * A new state needs an entry here. instance-state.test.mjs then checks that
 * app.css colours its badge with this token, and that the selected-row badge
 * uses the contrast-checked colour refreshSelectionTheme() computes for it.
 */
export const stateTones = {
  Listening: 'ok',
  Starting: 'warn',
  Queued: 'warn',
  Stopping: 'warn',
  PortOccupied: 'error',
  RunUnconfirmed: 'error',
  StateUnconfirmed: 'error',
  Stopped: 'muted',
} as const satisfies Record<string, 'ok' | 'warn' | 'error' | 'muted'>
