<script setup lang="ts">
import { open } from '@tauri-apps/plugin-dialog'
import { api } from '../api'
import { basename } from '../format'
import { loadConfig, refresh, selectRepo, settingsOpen, state } from '../store'

const emit = defineEmits<{ (e: 'error', message: string): void }>()

async function addRepo() {
  const picked = await open({ directory: true, multiple: false, title: 'Choose the main clone' })
  if (typeof picked !== 'string') return
  try {
    const repo = await api.addRepo(picked)
    await loadConfig()
    await selectRepo(repo.id)
  } catch (e) {
    emit('error', String(e))
  }
}

async function forget(id: string) {
  await api.removeRepo(id)
  await loadConfig()
  await refresh({ fetch: false })
}
</script>

<template>
  <aside class="pane sidebar">
    <div class="pane-body">
      <div class="section-title">Repositories</div>
      <button
        v-for="repo in state.config?.repos ?? []"
        :key="repo.id"
        class="repo-item"
        :class="{ selected: repo.id === state.config?.selectedRepoId }"
        @click="selectRepo(repo.id)"
        @contextmenu.prevent="forget(repo.id)"
      >
        <span class="repo-name truncate">{{ repo.name }}</span>
        <span class="repo-sub truncate">{{ basename(repo.path) }}</span>
      </button>

      <p v-if="!state.config?.repos.length" class="empty" style="padding: 20px 14px">
        Add the main clone of a repo to get started.
      </p>
    </div>

    <div class="pane-header" style="border-top: 1px solid var(--border-soft); border-bottom: none">
      <button class="btn ghost small" @click="addRepo">+ Add repo</button>
      <span class="spacer" />
      <button class="btn ghost small" title="Settings" @click="settingsOpen = true">⚙</button>
    </div>
  </aside>
</template>
