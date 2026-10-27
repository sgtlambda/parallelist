<script setup lang="ts">
import Icon from './Icon.vue'
import { addProject, state } from '../store'

/**
 * What the window shows while there is no project, in place of a project
 * list with nothing in it, a branch list that would say "Select a project",
 * a detail pane that would say "Pick a branch" and an attention queue with
 * nothing in it. It says what a project is, offers the one button that moves
 * things on, and shows what the window does once there is one. Its own error
 * banner, because a folder git rejects is reported in `state.error`, which
 * the branch list shows and this replaces.
 */
</script>

<template>
  <!-- A deep drag region: the text drags the window, the button still clicks. -->
  <main class="pane onboarding" data-tauri-drag-region="deep">
    <div class="welcome">
      <Icon name="logo" :size="40" class="logo" />
      <h1>Add a project</h1>
      <p class="lead">
        Choose the <strong>main workspace</strong> of a git repository: the clone you work in.
        parallelist keeps its branches, workspaces and terminals in one window.
      </p>

      <div v-if="state.error" class="banner error">{{ state.error }}</div>

      <div class="add-row">
        <button class="btn primary add" @click="addProject">Add project…</button>
        <span class="faint">or drop its folder on this window</span>
      </div>

      <ul class="then">
        <li>
          <Icon name="branch" />
          <span>Every branch on origin, with the workspaces that hold it and its pull request.</span>
        </li>
        <li>
          <Icon name="terminal" />
          <span>A terminal for each workspace, which keeps running while you look elsewhere.</span>
        </li>
        <li>
          <Icon name="check" />
          <span>
            A mark when something needs you: <b class="blocked">orange</b> while a program waits
            for an answer, <b class="ready">blue</b> when a finished turn hasn't been seen.
          </span>
        </li>
        <li>
          <Icon name="search" />
          <span>
            <kbd>⌘P</kbd> reaches any branch of any project, and <kbd>⌘↵</kbd> goes to whatever
            has waited longest.
          </span>
        </li>
      </ul>

      <p class="faint note">
        Pull request data comes from <code>gh</code>, once signed in with
        <code>gh auth login</code>. Everything else works without it.
      </p>
    </div>
  </main>
</template>

<style scoped>
.onboarding {
  /* Past the overlay title bar; the rest is centred. */
  padding-top: calc(var(--titlebar-h) + var(--build-h));
  align-items: center;
  justify-content: center;
  grid-column: 1 / -1;
  grid-row: 1;
}

.welcome {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  width: min(460px, 100%);
  padding: 24px;
  /* Scrolls rather than clips in a window too short for it. */
  max-height: 100%;
  overflow-y: auto;
}

.logo {
  color: var(--text-dim);
  margin-bottom: 14px;
}

h1 {
  margin: 0 0 8px;
  font-size: 20px;
  font-weight: 600;
}

.lead {
  margin: 0 0 18px;
  color: var(--text-dim);
  line-height: 1.55;
}

.lead strong {
  color: var(--text);
  font-weight: 600;
}

.banner {
  align-self: stretch;
  margin: 0 0 14px;
}

.add-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px 12px;
}

.add {
  padding: 6px 14px;
}

.then {
  list-style: none;
  margin: 28px 0 0;
  padding: 18px 0 0;
  border-top: 1px solid var(--border-soft);
  display: flex;
  flex-direction: column;
  gap: 12px;
  color: var(--text-dim);
  line-height: 1.5;
}

.then li {
  display: flex;
  gap: 10px;
}

.then li > :first-child {
  flex: none;
  margin-top: 3px;
  color: var(--text-faint);
}

.then b {
  font-weight: 600;
}

.then b.blocked {
  color: var(--orange);
}

.then b.ready {
  color: var(--accent);
}

kbd {
  font: inherit;
  font-size: 10.5px;
  line-height: 14px;
  padding: 0 4px;
  border: 1px solid var(--border);
  border-radius: 4px;
}

.note {
  margin: 18px 0 0;
  font-size: 11.5px;
  line-height: 1.5;
}
</style>
