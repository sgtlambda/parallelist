<script setup lang="ts">
import { computed, ref } from 'vue'
import { adoptTargets, useSuggestions } from '../../firstRun'
import { shortPath } from '../../format'

/**
 * Concept 1, a checklist. A dialog over the window on first launch, laid out
 * as Settings is: what was found on this Mac in groups, each with a switch,
 * the recommended ones on. One decision, one button.
 */
const emit = defineEmits<{ close: [] }>()
const { list, error, chosen, found, byCategory, toggle, picked } = useSuggestions()
const saving = ref(false)

const groups = computed(() => byCategory(found.value))
const count = computed(() => picked.value.length)

async function finish(add: boolean) {
  saving.value = true
  await adoptTargets(add ? picked.value : [])
  emit('close')
}
</script>

<template>
  <div class="scrim settings-scrim">
    <div class="modal fr" role="dialog" aria-labelledby="fr-title">
      <div class="modal-header" data-tauri-drag-region="deep">
        <span id="fr-title">Open workspaces in</span>
      </div>

      <div class="modal-body">
        <div v-if="error" class="banner error">{{ error }}</div>

        <template v-else-if="!list">
          <div class="set-group">
            <h3 class="faint">Looking for installed programs…</h3>
            <div class="set-list">
              <div v-for="i in 4" :key="i" class="set-row fr-row" :style="{ '--i': i }">
                <span class="shimmer bar" style="width: 18px" />
                <span class="set-row-text">
                  <span class="shimmer bar" :style="{ width: `${90 + i * 23}px`, display: 'block' }" />
                </span>
                <span class="shimmer pill" style="width: 32px; height: 19px" />
              </div>
            </div>
          </div>
        </template>

        <template v-else>
          <p v-if="!found.length" class="set-foot">Nothing found.</p>
          <section v-for="group in groups" :key="group.key" class="set-group">
            <h3 v-if="group.title">{{ group.title }}</h3>
            <div class="set-list">
              <label v-for="s in group.items" :key="s.target.id" class="set-row fr-row">
                <span class="fr-icon">{{ s.target.icon }}</span>
                <span class="set-row-text">
                  <span class="set-label">{{ s.target.label }}</span>
                  <p class="mono truncate">{{ shortPath(s.found!) }}</p>
                </span>
                <input
                  type="checkbox"
                  class="switch"
                  :checked="chosen.has(s.target.id)"
                  @change="toggle(s.target.id)"
                />
              </label>
            </div>
          </section>

        </template>
      </div>

      <div class="modal-footer">
        <button class="btn ghost" type="button" :disabled="saving" @click="finish(false)">Skip</button>
        <span class="spacer" />
        <button
          class="btn primary"
          type="button"
          :disabled="!list || !count || saving"
          @click="finish(true)"
        >
          {{ !list ? 'Add targets' : count === 1 ? 'Add 1 target' : `Add ${count} targets` }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.fr {
  max-width: 520px;
  height: min(640px, 100%);
}

.modal-body {
  flex: 1;
  gap: 18px;
}

.fr-row {
  gap: 12px;
  min-height: 44px;
  padding: 7px 14px;
  cursor: pointer;
}

.fr-row:hover {
  background: var(--bg-hover);
}

.fr-icon {
  flex: none;
  width: 18px;
  text-align: center;
  font-size: 14px;
}

.fr-row .set-row-text p {
  font-size: 11px;
  color: var(--text-faint);
}
</style>
