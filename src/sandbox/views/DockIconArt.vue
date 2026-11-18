<script setup lang="ts">
/**
 * `src-tauri/icons/icon.svg`, with the cursor drawn or not. Its gradient and
 * filters are defined once by `DockIcon.vue` (`#dock-body`, `#dock-shadow`,
 * `#dock-lift`), so every copy on the page shares them.
 */
defineProps<{ cursorOn: boolean; size: number }>()
</script>

<template>
  <svg :width="size" :height="size" viewBox="0 0 1024 1024" class="art">
    <path
      fill="url(#dock-body)"
      filter="url(#dock-shadow)"
      d="M627.36 100c103.83 0 155.75 0 195.41 20.21a185.4 185.4 0 0 1 81.02 81.02c20.21 39.66 20.21 91.58 20.21 195.41L924 627.36c0 103.83 0 155.75 -20.21 195.41a185.4 185.4 0 0 1 -81.02 81.02c-39.66 20.21 -91.58 20.21 -195.41 20.21L396.64 924c-103.83 0 -155.75 0 -195.41 -20.21a185.4 185.4 0 0 1 -81.02 -81.02c-20.21 -39.66 -20.21 -91.58 -20.21 -195.41L100 396.64c0 -103.83 0 -155.75 20.21 -195.41a185.4 185.4 0 0 1 81.02 -81.02c39.66 -20.21 91.58 -20.21 195.41 -20.21Z"
    />
    <g transform="translate(28 -13)">
      <rect x="350" y="250" width="444" height="340" rx="40" fill="#FFFFFF" fill-opacity="0.28" />
      <rect x="290" y="310" width="444" height="340" rx="40" fill="#FFFFFF" fill-opacity="0.5" />
      <rect x="230" y="370" width="444" height="340" rx="40" fill="#FFFFFF" filter="url(#dock-lift)" />
      <g fill="none" stroke="#4F46E5" stroke-width="26" stroke-linecap="round" stroke-linejoin="round">
        <path d="M348 607.5L417 538.5L348 469.5" />
        <path v-if="cursorOn" d="M440 630.5H532" />
      </g>
    </g>
  </svg>
</template>

<style scoped>
.art {
  display: block;
}
</style>
