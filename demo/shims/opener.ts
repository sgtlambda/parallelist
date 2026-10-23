import { showToast } from '../../src/store'

/** The pull requests here are made up, so there is nothing to open. */
export async function openUrl(): Promise<void> {
  showToast('Links are switched off in the demo')
}
