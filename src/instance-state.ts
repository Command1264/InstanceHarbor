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
