import { LogicalPosition } from '@tauri-apps/api/dpi'
import { Menu, type MenuItemOptions } from '@tauri-apps/api/menu'

/**
 * Native context menus: macOS's own, which can reach past the window's edge,
 * rather than a lookalike drawn in the page.
 */

export interface MenuItem {
  id: string
  label: string
  /** Draws a divider above this item. */
  separator?: boolean
  /** Shown but not choosable. */
  disabled?: boolean
  /** Turns the entry into a submenu rather than an action. */
  children?: MenuItem[]
}

/** The menu shown last. It is closed when the next one opens rather than
 *  when it is dismissed, since whether the chosen item's action arrives
 *  before or after `popup` resolves is the native side's business. */
let shown: Menu | null = null

/**
 * Pop up a menu, at the pointer or at `at` (window coordinates), and call
 * `onSelect` with the id of whatever is chosen.
 *
 * Items are passed as options, so the whole tree is one native resource.
 * Their ids are ours rather than generated: Tauri keeps every item's action
 * by id for good, so reusing ids is what stops that from growing with every
 * menu shown.
 */
export async function popupMenu(
  items: MenuItem[],
  onSelect: (id: string) => void,
  at?: { x: number; y: number },
) {
  const menu = await Menu.new({ items: build(items, onSelect) })
  void shown?.close()
  shown = menu
  await menu.popup(at && new LogicalPosition(at.x, at.y))
}

type Options = MenuItemOptions | { item: 'Separator' } | {
  id: string
  text: string
  enabled?: boolean
  items: Options[]
}

function build(items: MenuItem[], onSelect: (id: string) => void): Options[] {
  const built: Options[] = []
  for (const item of items) {
    if (item.separator && built.length) built.push({ item: 'Separator' })
    const id = `menu:${item.id}`
    const common = { id, text: item.label, enabled: !item.disabled }
    if (item.children) {
      built.push({ ...common, items: build(item.children, onSelect) })
    } else {
      built.push({ ...common, action: () => onSelect(item.id) })
    }
  }
  return built
}

