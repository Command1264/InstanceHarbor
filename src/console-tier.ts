// Explains the console tier the Host chose for a run (console-tier.json).
import { tr } from './i18n.ts'

export type ConsoleTier = 'bundled' | 'system' | 'window' | 'plain'
/** The console tier a run uses and every better tier it skipped, recorded by the Host. */
export interface ConsoleChoice { tier: ConsoleTier; skipped: { tier: ConsoleTier; reason: string }[] }
export interface ConsolePlan extends ConsoleChoice { needsConfirmation: boolean }
const consoleReasons = ['ConsoleRuntimeMissing', 'ConsoleRuntimeMismatch', 'ConsoleSystemUnavailable', 'ConsoleCoreUnsupported',
  'ConsolePathUnsupported', 'ConsoleWindowNotConfirmed', 'ConsoleTierLimited'] as const
export function consoleReason(reason: string) {
  const known = consoleReasons.find(code => code === reason)
  return known ? tr(`console.reason.${known}`) : tr('console.reason.other', { code: reason })
}
/** Why a run uses a lower tier: the reason the preferred InstanceHarbor console was skipped. */
export function consoleFallbackReason(choice: ConsoleChoice) {
  const skipped = choice.skipped.find(item => item.tier === 'bundled')
  if (!skipped) return null
  // A plain run that was never offered a window explains itself by the better tiers' reasons.
  const system = choice.skipped.find(item => item.tier === 'system')
  if (choice.tier === 'plain' && system && skipped.reason !== system.reason) return consoleReason(system.reason)
  return consoleReason(skipped.reason)
}
export function consoleSummary(choice: ConsoleChoice) {
  const tier = tr(`console.tier.${choice.tier}`)
  const reason = choice.tier === 'bundled' ? null : consoleFallbackReason(choice)
  return reason ? tr('console.tier.because', { tier, reason }) : tier
}
