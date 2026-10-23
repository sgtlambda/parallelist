import { LogicalPosition } from './dpi'

/**
 * `@tauri-apps/api/menu`: macOS draws these natively, which a web page can't,
 * so this draws a lookalike. Only what `src/menu.ts` builds is supported:
 * actions, separators and one level of submenu.
 */

interface Entry {
  id?: string
  text?: string
  enabled?: boolean
  item?: 'Separator'
  action?: () => void
  items?: Entry[]
}

let pointer = { x: 0, y: 0 }
window.addEventListener(
  'pointermove',
  (e) => void (pointer = { x: e.clientX, y: e.clientY }),
  { capture: true, passive: true },
)
window.addEventListener('contextmenu', (e) => void (pointer = { x: e.clientX, y: e.clientY }), {
  capture: true,
})

export class Menu {
  private close: (() => void) | null = null

  private constructor(private items: Entry[]) {}

  static async new(options: { items: Entry[] }): Promise<Menu> {
    return new Menu(options.items)
  }

  async popup(at?: LogicalPosition): Promise<void> {
    this.close?.()
    const { x, y } = at ?? pointer
    return new Promise((resolve) => {
      const backdrop = document.createElement('div')
      // Named as App.vue's type-to-search expects: a menu owns the keyboard.
      backdrop.className = 'menu-backdrop demo-menu-backdrop'
      const root = this.render(this.items, () => done())
      backdrop.appendChild(root)
      document.body.appendChild(backdrop)

      const box = root.getBoundingClientRect()
      root.style.left = `${Math.max(4, Math.min(x, window.innerWidth - box.width - 4))}px`
      root.style.top = `${Math.max(4, Math.min(y, window.innerHeight - box.height - 4))}px`

      const onKey = (e: KeyboardEvent) => {
        if (e.key !== 'Escape') return
        e.preventDefault()
        e.stopPropagation()
        done()
      }
      const done = () => {
        window.removeEventListener('keydown', onKey, true)
        backdrop.remove()
        this.close = null
        resolve()
      }
      this.close = done
      window.addEventListener('keydown', onKey, true)
      backdrop.addEventListener('pointerdown', (e) => {
        if (e.target === backdrop) done()
      })
      backdrop.addEventListener('contextmenu', (e) => {
        e.preventDefault()
        if (e.target === backdrop) done()
      })
    })
  }

  private render(items: Entry[], choose: () => void): HTMLElement {
    const list = document.createElement('div')
    list.className = 'demo-menu'
    for (const entry of items) {
      if (entry.item === 'Separator') {
        list.appendChild(Object.assign(document.createElement('hr'), { className: 'demo-menu-rule' }))
        continue
      }
      const row = document.createElement('div')
      row.className = 'demo-menu-item'
      if (entry.enabled === false) row.classList.add('disabled')
      row.textContent = entry.text ?? ''
      if (entry.items) {
        row.classList.add('has-children')
        const sub = this.render(entry.items, choose)
        sub.classList.add('demo-submenu')
        row.appendChild(sub)
      } else {
        row.addEventListener('click', (e) => {
          e.stopPropagation()
          if (entry.enabled === false) return
          choose()
          entry.action?.()
        })
      }
      list.appendChild(row)
    }
    return list
  }
}

export type MenuItemOptions = Entry
