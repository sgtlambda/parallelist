<script setup lang="ts">
import { computed, ref } from 'vue'
import Icon from './Icon.vue'
import { adoptTargets, useSuggestions } from '../firstRun'

/**
 * The onboarding's first step on a new config: the "Open in" targets to
 * start with, as a tile per program `suggest_targets` found on this Mac,
 * the ticked ones lit. Nothing that wasn't found is shown; Settings is
 * where anything else is added. Continue adopts the lit ones, Skip none,
 * and either way the choice is made and the onboarding moves on.
 */
const emit = defineEmits<{ done: [] }>()
const { list, error, chosen, found, byCategory, toggle, picked } = useSuggestions()
const saving = ref(false)

// Two groups: a tile says enough on its own. Finder leads the apps.
const groups = computed(() => {
  const all = byCategory(found.value)
  const take = (keys: string[]) =>
    keys.flatMap((key) => all.find((g) => g.key === key)?.items ?? [])
  return [
    { key: 'agent', title: 'In the terminal', items: take(['agent']) },
    { key: 'apps', title: 'Apps', items: take(['other', 'editor', 'terminal', 'git']) },
  ].filter((g) => g.items.length)
})

async function next(add: boolean) {
  saving.value = true
  try {
    await adoptTargets(add ? picked.value : [])
    emit('done')
  } catch (e) {
    error.value = String(e)
    saving.value = false
  }
}
</script>

<template>
  <h1>Open workspaces in</h1>
  <p class="lead">Each one picked is offered on every workspace, to open it there with a click.</p>

  <div v-if="error" class="banner error">{{ error }}</div>

  <div v-else-if="!list" class="tiles">
    <div v-for="i in 6" :key="i" class="tile ghost" :style="{ '--i': i }">
      <span class="shimmer bar" style="width: 22px; height: 22px; border-radius: 6px" />
      <span class="shimmer bar" style="width: 70%" />
    </div>
  </div>

  <template v-else>
    <p v-if="!found.length" class="faint">Nothing found.</p>
    <section v-for="group in groups" :key="group.key" class="group">
      <h2>{{ group.title }}</h2>
      <div class="tiles">
        <button
          v-for="s in group.items"
          :key="s.target.id"
          type="button"
          class="tile"
          :class="{ on: chosen.has(s.target.id) }"
          :aria-pressed="chosen.has(s.target.id)"
          :title="s.found ?? undefined"
          @click="toggle(s.target.id)"
        >
          <span class="glyph">{{ s.target.icon }}</span>
          <span class="name">{{ s.target.label }}</span>
          <span class="tick"><Icon name="check" :size="10" /></span>
        </button>
      </div>
    </section>
  </template>

  <div class="next">
    <button class="btn primary" :disabled="!list || saving" @click="next(true)">Continue</button>
    <button class="btn ghost" :disabled="saving" @click="next(false)">Skip</button>
  </div>
</template>

<style scoped>
h1 {
  margin: 0 0 8px;
  font-size: 20px;
  font-weight: 600;
}

.lead {
  margin: 0 0 20px;
  color: var(--text-dim);
  line-height: 1.55;
}

.banner {
  align-self: stretch;
}

.group {
  align-self: stretch;
}

.group + .group {
  margin-top: 18px;
}

h2 {
  margin: 0 0 8px;
  font-size: 11.5px;
  font-weight: 600;
  color: var(--text-dim);
}

.tiles {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(112px, 1fr));
  gap: 8px;
  align-self: stretch;
}

.tile {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  min-width: 0;
  padding: 11px 12px 10px;
  border: 1px solid var(--border-soft);
  border-radius: 10px;
  background: var(--bg-raised);
  text-align: left;
  color: var(--text-dim);
  cursor: pointer;
  transition:
    border-color 0.12s ease,
    background 0.12s ease;
}

.tile:hover {
  border-color: var(--border);
}

.tile.on {
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 7%, var(--bg-raised));
  color: var(--text);
}

.tile.ghost {
  gap: 10px;
  height: 64px;
  cursor: default;
}

.glyph {
  font-size: 20px;
  line-height: 26px;
  margin-bottom: 4px;
  filter: grayscale(1);
  opacity: 0.6;
}

.tile.on .glyph {
  filter: none;
  opacity: 1;
}

.name {
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 550;
}

.tick {
  position: absolute;
  top: 8px;
  right: 8px;
  display: grid;
  place-items: center;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  border: 1px solid var(--border);
  color: transparent;
}

.tile.on .tick {
  background: var(--accent);
  border-color: transparent;
  color: var(--accent-fg);
}

.next {
  display: flex;
  align-items: center;
  gap: 8px;
  align-self: stretch;
  margin-top: 24px;
  padding-top: 16px;
  border-top: 1px solid var(--border-soft);
}

.next .btn.primary {
  padding: 6px 16px;
}
</style>
