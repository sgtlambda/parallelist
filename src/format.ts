/** "2d", "3h", "just now" — the branch list needs this to stay narrow. */
export function relativeTime(unixSeconds: number): string {
  if (!unixSeconds) return ''
  const seconds = Math.max(0, Date.now() / 1000 - unixSeconds)
  if (seconds < 90) return 'just now'
  const minutes = seconds / 60
  if (minutes < 60) return `${Math.round(minutes)}m`
  const hours = minutes / 60
  if (hours < 24) return `${Math.round(hours)}h`
  const days = hours / 24
  if (days < 31) return `${Math.round(days)}d`
  const months = days / 30.4
  if (months < 12) return `${Math.round(months)}mo`
  return `${Math.round(months / 12)}y`
}

export function bytes(value: number): string {
  if (!value) return '—'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let n = value
  let i = 0
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024
    i++
  }
  return `${n < 10 && i > 0 ? n.toFixed(1) : Math.round(n)} ${units[i]}`
}

let homeDirectory = ''

/** Called once at startup. Until then paths simply render in full. */
export function setHomeDirectory(path: string) {
  homeDirectory = path.replace(/\/+$/, '')
}

/** Paths are long and the interesting part is the tail, so `/Users/you/Code/x`
 *  renders as `~/Code/x`. */
export function shortPath(path: string): string {
  return homeDirectory && path.startsWith(`${homeDirectory}/`)
    ? `~${path.slice(homeDirectory.length)}`
    : path
}

export function basename(path: string): string {
  return path.split('/').filter(Boolean).pop() ?? path
}

/** Human names for workspace kinds, used as icon tooltips. */
export const WORKSPACE_LABELS: Record<string, string> = {
  primary: 'Primary clone',
  worktree: 'Worktree',
  clone: 'Full clone',
  orphan: 'Orphaned directory — not a git workspace',
}
