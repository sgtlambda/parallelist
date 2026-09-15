<script lang="ts">
/**
 * Placeholder icon set.
 *
 * Every glyph is inline SVG on a 24x24 grid, stroked with `currentColor` so it
 * inherits the surrounding text colour and weight. The point of this file is
 * that it is the *only* place glyphs are defined: call sites ask for a
 * semantic name ("worktree"), never a specific picture. Swapping to a real
 * icon set later means rewriting this map and nothing else.
 */
export const GLYPHS = {
  // git-branch: a trunk with a branch peeling off — a linked working copy.
  worktree:
    '<line x1="6" x2="6" y1="3" y2="15"/>' +
    '<circle cx="18" cy="6" r="3"/>' +
    '<circle cx="6" cy="18" r="3"/>' +
    '<path d="M18 9a9 9 0 0 1-9 9"/>',

  // copy: two sheets, one behind the other — an independent second copy.
  clone:
    '<rect width="14" height="14" x="8" y="8" rx="2" ry="2"/>' +
    '<path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>',

  // home: the main workspace everything else hangs off.
  primary:
    '<path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>' +
    '<polyline points="9 22 9 12 15 12 15 22"/>',

  // folder with a cross: a directory that is no longer a workspace at all.
  orphan:
    '<path d="M4 5h4.6l1.8 2.2H20a1 1 0 0 1 1 1V18a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1Z"/>' +
    '<path d="m9.6 10.7 4.8 4.8"/>' +
    '<path d="m14.4 10.7-4.8 4.8"/>',
  // check: something finished and worked. One stroke, 22.6 units long, so it
  // can be drawn in with a dash of 23 (see `.icon-draw`).
  check: '<path d="M20 6 9 17l-5-5"/>',

  // vertical ellipsis: the "more actions" affordance.
  more:
    '<circle cx="12" cy="5" r="1.5" fill="currentColor" stroke="none"/>' +
    '<circle cx="12" cy="12" r="1.5" fill="currentColor" stroke="none"/>' +
    '<circle cx="12" cy="19" r="1.5" fill="currentColor" stroke="none"/>',
} as const

export type IconName = keyof typeof GLYPHS
</script>

<script setup lang="ts">
import { computed } from 'vue'

const props = withDefaults(defineProps<{ name: IconName; size?: number }>(), {
  size: 13,
})

const glyph = computed(() => GLYPHS[props.name])
</script>

<template>
  <svg
    class="icon"
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    focusable="false"
    v-html="glyph"
  />
</template>
