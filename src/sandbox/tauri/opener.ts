import { log } from '../backend'

export function openUrl(url: string): Promise<void> {
  log('open', 'openUrl', url)
  return Promise.resolve()
}
