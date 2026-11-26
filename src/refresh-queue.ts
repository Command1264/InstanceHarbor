export type RefreshOrigin = 'manual' | 'background'

/**
 * Whether a list refresh asked for while another one is still reading runs right
 * after it. A background tick does not: when a read takes longer than the 1.5 s poll,
 * queuing every tick made the GUI ask the Manager again the moment each reply
 * arrived, so a slow Manager never got a pause (#118). The next tick reads anyway.
 */
export function queuesBehindActive(origin: RefreshOrigin): boolean {
  return origin === 'manual'
}
