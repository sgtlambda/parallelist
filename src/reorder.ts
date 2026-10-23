import { nextTick, watch, type Ref } from 'vue'

/**
 * Slides a list's rows from where they were to where they are whenever its
 * order changes, so a row that moved can be followed by eye: measure, let
 * Vue move the elements, measure again, and animate the difference away
 * (FLIP). A row that is new fades in; one that went just goes, and the rows
 * after it slide up into its place.
 *
 * Only within one listing (`listing`, a key for it): another tab, search or
 * project is a different list, and rows sliding there would suggest they
 * had moved when they hadn't.
 *
 * Hand-rolled on the Web Animations API rather than motion-v's `layout`:
 * that animates every layout change, which a tab switch or a keystroke in
 * the search is too, and the branch list can be hundreds of rows. Here a
 * list that didn't change order costs nothing, and one that did reads each
 * row's position twice.
 *
 * Rows are the container's descendants with a `data-reorder` attribute,
 * whose value is the row's key.
 */
export function animateReorder(
  container: Ref<HTMLElement | null>,
  keys: () => string[],
  listing: () => string,
) {
  // Two sources rather than one getter returning both, which would be a new
  // array, and so a change, every time the rows are recomputed at all.
  watch(
    [() => keys().join('\n'), listing],
    ([, now], [, was]) => {
      const el = container.value
      if (!el || now !== was || reducedMotion()) return
      // `pre`: this runs before the component patches the DOM, so these are
      // the old positions, mid-slide ones included.
      const before = new Map([...rowsOf(el)].map(([key, row]) => [key, row.getBoundingClientRect().top]))
      void nextTick(() => play(el, before))
    },
    { flush: 'pre' },
  )
}

const ID = 'reorder'
/** Subtle: long enough to see where a row went, short enough not to wait for. */
const DURATION = 280
const EASING = 'cubic-bezier(0.2, 0, 0, 1)'

/** Rows mid-slide that were raised above the rest, by the slide that did it. */
const lifted = new WeakMap<HTMLElement, Animation>()

function play(container: HTMLElement, before: Map<string, number>) {
  const rows = rowsOf(container)
  // Interrupted slides stop where they are: `before` measured them there,
  // so the next slide carries on from that point instead of jumping.
  for (const row of rows.values()) {
    row.getAnimations().forEach((a) => a.id === ID && a.cancel())
    unlift(row)
  }

  // Every read before any write, or each animation would force a layout.
  const view = container.getBoundingClientRect()
  const moves = [...rows].map(([key, row]) => {
    const rect = row.getBoundingClientRect()
    const first = before.get(key)
    if (first === undefined) return { row, rect, from: null, to: rect.top }
    // Where it is seen to go from and to. A row coming from far out of sight
    // starts just past the edge, and one going far ends just past it, so it
    // slides rather than flashing across at many times the speed. Past the
    // edge, the jump to where it really is can't be seen.
    const edges = (top: number) => Math.max(view.top - rect.height, Math.min(view.bottom, top))
    return { row, rect, from: edges(first), to: edges(rect.top) }
  })
  const distance = (m: (typeof moves)[number]) => (m.from === null ? 0 : Math.abs(m.from - m.to))
  const furthest = Math.max(0, ...moves.map(distance))
  let backdrop: string | null = null

  for (const move of moves) {
    const { row, rect, from, to } = move
    if (from === null) {
      if (inView(view, rect.top, rect.height)) {
        row.animate([{ opacity: 0 }, { opacity: 1 }], { id: ID, duration: DURATION, easing: 'ease-out' })
      }
      continue
    }
    // Unmoved, or out of sight throughout.
    if (distance(move) < 1) continue

    const animation = row.animate(
      [{ transform: `translateY(${from - rect.top}px)` }, { transform: `translateY(${to - rect.top}px)` }],
      { id: ID, duration: DURATION, easing: EASING },
    )
    // The row that went furthest passes over the ones making room for it.
    // Raised and backed, or the rows' text runs through each other on the
    // way. Two rows trading places both are, so one covers the other.
    if (distance(move) > furthest - 1 && furthest > rect.height - 1) {
      backdrop ??= backdropOf(container)
      lift(row, animation, backdrop)
    }
  }
}

function rowsOf(container: HTMLElement): Map<string, HTMLElement> {
  const rows = container.querySelectorAll<HTMLElement>('[data-reorder]')
  return new Map([...rows].map((row) => [row.dataset.reorder!, row]))
}

function inView(view: DOMRect, top: number, height: number): boolean {
  return top < view.bottom && top + height > view.top
}

function lift(row: HTMLElement, animation: Animation, backdrop: string) {
  lifted.set(row, animation)
  row.style.position = 'relative'
  row.style.zIndex = '1'
  // A selected row has a background of its own.
  if (!row.classList.contains('selected')) row.style.backgroundColor = backdrop
  // `finish` is dispatched later, by when a newer slide may have lifted it.
  animation.addEventListener('finish', () => lifted.get(row) === animation && unlift(row))
}

function unlift(row: HTMLElement) {
  if (!lifted.delete(row)) return
  row.style.removeProperty('position')
  row.style.removeProperty('z-index')
  row.style.removeProperty('background-color')
}

/** The colour the list is drawn on: the nearest background that isn't
 *  transparent, since the rows have none of their own. */
function backdropOf(el: HTMLElement): string {
  for (let e: HTMLElement | null = el; e; e = e.parentElement) {
    const color = getComputedStyle(e).backgroundColor
    if (color && color !== 'transparent' && !/[,/]\s*0\)$/.test(color)) return color
  }
  return 'Canvas'
}

function reducedMotion(): boolean {
  return window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false
}
