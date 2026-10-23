import { LogicalPosition } from './dpi'

/**
 * A native context menu, drawn in the page. Only the shape the app uses:
 * `Menu.new({ items })`, `popup(at?)` and `close()`, with items that are
 * actions, separators or submenus.
 */

type Item =
  | { item: 'Separator' }
  | { id: string; text: string; enabled?: boolean; action: () => void }
  | { id: string; text: string; enabled?: boolean; items: Item[] }

let pointer = { x: 0, y: 0 }
document.addEventListener('pointerdown', (e) => (pointer = { x: e.clientX, y: e.clientY }), true)

function list(items: Item[], close: () => void): HTMLElement {
  const el = document.createElement('div')
  el.dataset.sandboxMenu = ''
  el.style.cssText = [
    'position:fixed',
    'min-width:180px',
    'padding:4px',
    'background:var(--bg-raised)',
    'border:1px solid var(--border)',
    'border-radius:8px',
    'box-shadow:var(--shadow)',
    'font-size:13px',
    'color:var(--text)',
    'z-index:2147483647',
  ].join(';')
  for (const item of items) {
    if ('item' in item) {
      const hr = document.createElement('div')
      hr.style.cssText = 'height:1px;margin:4px 6px;background:var(--border-soft)'
      el.append(hr)
      continue
    }
    const row = document.createElement('div')
    const enabled = item.enabled !== false
    row.style.cssText = `display:flex;justify-content:space-between;gap:16px;padding:4px 10px;border-radius:5px;${
      enabled ? '' : 'opacity:.4'
    }`
    row.textContent = item.text
    let sub: HTMLElement | null = null
    row.addEventListener('pointerenter', () => {
      sub?.remove()
      if (enabled) {
        row.style.background = 'var(--accent)'
        row.style.color = 'var(--accent-fg)'
      }
      if (enabled && 'items' in item) {
        sub = list(item.items, close)
        document.body.append(sub)
        const r = row.getBoundingClientRect()
        sub.style.left = `${r.right + 2}px`
        sub.style.top = `${r.top - 4}px`
      }
    })
    row.addEventListener('pointerleave', (e) => {
      row.style.background = ''
      row.style.color = ''
      // Moving into the flyout keeps it.
      if (!(e.relatedTarget instanceof Node && sub?.contains(e.relatedTarget))) sub?.remove()
    })
    if (enabled && 'action' in item) {
      row.addEventListener('click', () => {
        close()
        item.action()
      })
    }
    el.append(row)
  }
  return el
}

export class Menu {
  private done: (() => void) | null = null

  private constructor(private items: Item[]) {}

  static new(options: { items: Item[] }): Promise<Menu> {
    return Promise.resolve(new Menu(options.items))
  }

  popup(at?: LogicalPosition): Promise<void> {
    this.close()
    const root = list(this.items, () => this.close())
    document.body.append(root)
    const { x, y } = at ?? pointer
    root.style.left = `${Math.min(x, window.innerWidth - root.offsetWidth - 4)}px`
    root.style.top = `${Math.min(y, window.innerHeight - root.offsetHeight - 4)}px`
    const away = (e: Event) => {
      // Flyouts are separate elements; anything outside every menu dismisses.
      if ((e.target as HTMLElement).closest?.('[data-sandbox-menu]')) {
        document.addEventListener('pointerdown', away, { once: true, capture: true })
        return
      }
      this.close()
    }
    setTimeout(() => document.addEventListener('pointerdown', away, { once: true, capture: true }))
    return new Promise((resolve) => (this.done = resolve))
  }

  close(): Promise<void> {
    document.querySelectorAll('body > [data-sandbox-menu]').forEach((e) => e.remove())
    this.done?.()
    this.done = null
    return Promise.resolve()
  }
}
