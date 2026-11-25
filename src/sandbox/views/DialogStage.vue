<script setup lang="ts">
import { onMounted } from 'vue'
import CleanupDialog from '../../components/CleanupDialog.vue'
import CommandPalette from '../../components/CommandPalette.vue'
import ConfirmQuitDialog from '../../components/ConfirmQuitDialog.vue'
import CreateDialog from '../../components/CreateDialog.vue'
import { paletteOpen } from '../../palette'

/**
 * One of the app's dialogs over a stand-in for the window, so a translucent
 * design has something behind it to show through. The design is a class on
 * the root element (`dlg-<name>`) that `dialogDesigns.css` keys on; the
 * dialogs themselves are the ones the app runs. Sandbox-only.
 */
const props = defineProps<{
  design?: string
  dialog: 'create' | 'picker' | 'quit' | 'quit-alone' | 'cleanup' | 'palette'
  open?: boolean
  // Passed through to the dialog, as its own scenario would.
  initialBranch?: string
  title?: string
  paths?: string[]
  branches?: string[]
}>()
const emit = defineEmits<{ (e: 'close'): void }>()

const root = document.documentElement
for (const name of Array.from(root.classList)) if (name.startsWith('dlg-')) root.classList.remove(name)
if (props.design) root.classList.add(`dlg-${props.design}`)

onMounted(() => {
  if (props.dialog === 'palette') paletteOpen.value = true
  if (props.dialog === 'picker') {
    // Typing is what opens the picker's list; do it as a user would.
    const typeInto = () => {
      const input = document.querySelector<HTMLInputElement>('.picker-field input')
      if (!input) return void setTimeout(typeInto, 50)
      input.focus()
      input.value = 'fix'
      input.dispatchEvent(new Event('input', { bubbles: true }))
    }
    setTimeout(typeInto, 150)
  }
})

const rows = [
  ['main', 'app', true],
  ['fix/1234-header', 'app-1234-fix-header', true],
  ['fix/billing-rounding', 'app-billing', true],
  ['feature/sso', 'app-b', false],
  ['feature/search-index', '', false],
  ['chore/deps', '', false],
  ['feature/audit-log', 'app-audit-log', false],
  ['fix/timezone-parse', '', false],
] as const
</script>

<template>
  <div class="stand-in" aria-hidden="true">
    <aside class="si-sidebar">
      <div class="si-lights"><i /><i /><i /></div>
      <div class="si-project on">app</div>
      <div class="si-project">billing</div>
      <div class="si-project">site</div>
      <div class="si-project">infra</div>
    </aside>
    <section class="si-list">
      <div class="si-tabs"><b>Active</b><span>Open PRs</span><span>Assigned</span><span>All</span></div>
      <div v-for="[branch, dir, active] in rows" :key="branch" class="si-row" :class="{ sel: branch === 'fix/billing-rounding' }">
        <span class="si-dot" :class="{ active }" />
        <span class="si-name">{{ branch }}</span>
        <span class="si-dir">{{ dir }}</span>
      </div>
    </section>
    <section class="si-term">
      <div><span class="g">❯</span> bundle exec rspec spec/models/invoice_spec.rb</div>
      <div class="d">Randomized with seed 41027</div>
      <div><span class="g">.........................</span><span class="r">F</span><span class="g">..........</span></div>
      <div class="r">Failures:</div>
      <div>&nbsp;&nbsp;1) Invoice#total rounds half-even</div>
      <div class="d">&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;expected: 10.02</div>
      <div class="d">&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;got: 10.03</div>
      <div class="y">36 examples, 1 failure</div>
      <div><span class="g">❯</span> <span class="b">git</span> diff --stat</div>
      <div>&nbsp;app/models/invoice.rb&nbsp;&nbsp;&nbsp;| 4 <span class="g">++</span><span class="r">--</span></div>
      <div>&nbsp;spec/models/invoice_spec.rb | 9 <span class="g">+++++++</span><span class="r">--</span></div>
    </section>
    <footer class="si-queue">
      <div class="si-card blocked">app-1234-fix-header<small>Run bundle exec rspec spec/models</small></div>
      <div class="si-card ready">app-b<small>Should the SAML assertion be signed?</small></div>
    </footer>
  </div>

  <CreateDialog
    v-if="dialog === 'create' || dialog === 'picker'"
    :open="!!open"
    :initial-branch="initialBranch ?? ''"
    @close="emit('close')"
  />
  <ConfirmQuitDialog
    v-else-if="dialog === 'quit' || dialog === 'quit-alone'"
    :ask="open ? { running: 5, waiting: 2, persistent: dialog === 'quit' } : null"
    @close="emit('close')"
  />
  <CleanupDialog
    v-else-if="dialog === 'cleanup'"
    :open="!!open"
    :title="title ?? 'Cleanup workspaces'"
    :paths="paths ?? []"
    :branches="branches ?? []"
    @close="emit('close')"
  />
  <CommandPalette v-else-if="dialog === 'palette'" />
</template>

<style scoped>
.stand-in {
  position: absolute;
  inset: 0;
  display: grid;
  grid-template-columns: 180px 300px 1fr;
  grid-template-rows: 1fr 64px;
  background: var(--bg);
  font-size: 12.5px;
  overflow: hidden;
}
.si-sidebar {
  grid-row: 1 / -1;
  background: var(--bg-sidebar);
  border-right: 1px solid var(--border-soft);
  padding: 14px 10px;
}
.si-lights {
  display: flex;
  gap: 8px;
  margin: 0 0 22px 4px;
}
.si-lights i {
  width: 12px;
  height: 12px;
  border-radius: 50%;
  background: #ff5f57;
}
.si-lights i + i {
  background: #febc2e;
}
.si-lights i + i + i {
  background: #28c840;
}
.si-project {
  padding: 5px 8px;
  border-radius: 6px;
  color: var(--text-dim);
}
.si-project.on {
  background: var(--bg-active);
  color: var(--text);
  font-weight: 600;
}
.si-list {
  border-right: 1px solid var(--border-soft);
  padding: 44px 8px 0;
}
.si-tabs {
  display: flex;
  gap: 12px;
  padding: 0 8px 10px;
  color: var(--text-dim);
  font-size: 11.5px;
}
.si-tabs b {
  color: var(--text);
}
.si-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 8px;
  border-radius: 6px;
}
.si-row.sel {
  background: color-mix(in srgb, var(--accent) 16%, transparent);
}
.si-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  border: 1px solid var(--text-faint);
}
.si-dot.active {
  background: var(--green);
  border-color: var(--green);
}
.si-name {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.si-dir {
  color: var(--text-faint);
  font-size: 11px;
}
.si-term {
  background: #141416;
  color: #d8d8dc;
  padding: 50px 18px;
  font: 12px/1.7 'SF Mono', ui-monospace, Menlo, monospace;
}
.si-term .g {
  color: #4ac26b;
}
.si-term .r {
  color: #f2695f;
}
.si-term .y {
  color: #d4a72c;
}
.si-term .b {
  color: #6aa5ff;
}
.si-term .d {
  color: #8a8a92;
}
.si-queue {
  grid-column: 2 / -1;
  display: flex;
  gap: 6px;
  padding: 6px;
  border-top: 1px solid var(--border-soft);
  background: var(--bg-sidebar);
}
.si-card {
  width: 240px;
  padding: 6px 10px;
  border-radius: 7px;
  font-weight: 600;
  display: flex;
  flex-direction: column;
}
.si-card small {
  font-weight: 400;
  color: var(--text-dim);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.si-card.blocked {
  background: color-mix(in srgb, var(--orange) 18%, transparent);
}
.si-card.ready {
  background: color-mix(in srgb, var(--accent) 16%, transparent);
}
</style>
