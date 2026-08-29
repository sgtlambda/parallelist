<script lang="ts">
export interface MenuItem {
  id: string
  label: string
  /** Draws a divider above this item. */
  separator?: boolean
  danger?: boolean
}
</script>

<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'
import { useEscape } from '../useEscape'

const props = withDefaults(
  defineProps<{
    open: boolean
    x: number
    y: number
    items: MenuItem[]
    /** 'right' hangs the menu's right edge off x, for a toolbar dropdown. */
    align?: 'left' | 'right'
  }>(),
  { align: 'left' },
)
const emit = defineEmits<{ (e: 'close'): void; (e: 'select', id: string): void }>()

useEscape(() => props.open, () => emit('close'))

const el = ref<HTMLElement | null>(null)
const pos = ref({ left: 0, top: 0 })

watch(
  () => [props.open, props.x, props.y],
  async () => {
    if (!props.open) return
    // Place it at the cursor, then pull it back inside the window once its
    // real size is known.
    pos.value = { left: props.x, top: props.y }
    await nextTick()
    const menu = el.value
    if (!menu) return
    const margin = 8
    const { offsetWidth: w, offsetHeight: h } = menu
    const wanted = props.align === 'right' ? props.x - w : props.x
    pos.value = {
      left: Math.max(margin, Math.min(wanted, window.innerWidth - w - margin)),
      top: Math.max(margin, Math.min(props.y, window.innerHeight - h - margin)),
    }
  },
)

function choose(item: MenuItem) {
  emit('select', item.id)
  emit('close')
}
</script>

<template>
  <Teleport to="body">
    <!-- A full-window backdrop makes "click anywhere else to dismiss" work
         without a document-level listener that would race the opening click. -->
    <div
      v-if="open"
      class="menu-backdrop"
      @click="emit('close')"
      @contextmenu.prevent="emit('close')"
      @wheel="emit('close')"
    >
      <div
        ref="el"
        class="menu"
        :style="{ left: `${pos.left}px`, top: `${pos.top}px` }"
        @click.stop
      >
        <template v-for="item in items" :key="item.id">
          <div v-if="item.separator" class="menu-separator" />
          <button
            class="menu-item"
            :class="{ danger: item.danger }"
            @click="choose(item)"
          >
            {{ item.label }}
          </button>
        </template>
      </div>
    </div>
  </Teleport>
</template>
