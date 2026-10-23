import { log } from '../backend'

/** There is no native picker in a browser: it reports being cancelled. */
export function open(options?: unknown): Promise<string | null> {
  log('open', 'dialog.open', options)
  return Promise.resolve(null)
}
