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

/** relativeTime() as a phrase: "2d ago", but "just now" rather than "just now ago". */
export function ago(unixSeconds: number): string {
  const time = relativeTime(unixSeconds)
  return !time || time === 'just now' ? time : `${time} ago`
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

/** What each kind of workspace actually is, spelled out. Used as tooltips,
 *  where there is room to say more than the one-word name. */
export const WORKSPACE_LABELS: Record<string, string> = {
  primary: 'Main workspace — the clone every worktree hangs off',
  worktree: 'Worktree — shares the main workspace\'s object store',
  clone: 'Clone — an independent repository of its own',
  orphan: 'Orphaned directory — not a workspace at all',
}

/** The heading a workspace card carries. Every kind here is a workspace; the
 *  suffix is what says which of the two kinds it is, so the pair is visible
 *  without hovering anything. */
export const WORKSPACE_TITLES: Record<string, string> = {
  primary: 'Main workspace',
  worktree: 'Workspace · worktree',
  clone: 'Workspace · clone',
  orphan: 'Orphaned directory',
}
