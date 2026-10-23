import type { Component } from 'vue'

/**
 * A scenario is one named state of one piece of UI: some seeded state and
 * the component to show it in. Files in `scenarios/` each default-export a
 * group; the page around the frame lists them and the frame mounts one.
 */
export interface Scenario {
  /** Unique within its group; it is the end of the URL. */
  id: string
  title: string
  /** What it shows, and why that state is worth looking at. */
  note?: string
  component: Component
  props?: Record<string, unknown>
  /** `fill` (default) gives the component the whole frame; `box` puts it in
   *  a bordered box of `size`, for the panes that expect a column. */
  layout?: 'fill' | 'box'
  size?: { width: number; height: number }
  /** Seeds the stores and the backend's answers. Runs before the component
   *  is mounted, and once per load of the frame: a reload is a reset. */
  setup?: () => void | Promise<void>
}

export interface Group {
  name: string
  scenarios: Scenario[]
}

export const defineGroup = (name: string, scenarios: Scenario[]): Group => ({ name, scenarios })
