import SettingsDialog from '../../components/SettingsDialog.vue'
import type { ProjectFile } from '../../api'
import { backend } from '../backend'
import { boot } from '../seed'
import { defineGroup } from '../define'
import { config, project, targets } from '../fixtures'

const steps = [
  {
    id: 'env',
    label: 'Copy untracked env files from the main workspace',
    defaultEnabled: true,
    script: 'for f in .env .env.local; do [ -e {primary}/$f ] && cp {primary}/$f .; done',
  },
  {
    id: 'deps',
    label: 'Symlink dependencies from the main workspace',
    defaultEnabled: false,
    script: 'for d in vendor node_modules; do [ -e {primary}/$d ] && ln -sfn {primary}/$d .; done',
  },
  { id: 'composer', label: 'composer install', defaultEnabled: false, script: 'composer install' },
  { id: 'pnpm', label: 'pnpm install', defaultEnabled: true, script: 'pnpm install' },
]

const settingsConfig = (selected: string | null) =>
  config({
    selectedProjectId: selected,
    openTargets: targets,
    projects: [
      project('app', {
        pathTemplate: '{parent}/{project}-{slug}',
        baseBranch: null,
        setupSteps: steps,
        openTargets: [
          {
            id: 'docker',
            label: 'Docker shell',
            icon: '🐳',
            command: 'docker compose exec app bash',
            kind: 'terminal',
            primary: false,
            runOnCreate: false,
          },
        ],
      }),
      project('parallelist'),
      project('docs'),
      project('billing-service-with-a-long-name'),
    ],
  })

async function open(selected: string | null, files: Record<string, ProjectFile> = {}, click?: string) {
  await boot({ config: settingsConfig(selected) })
  backend.handle({ project_files: () => files, save_config: () => {} })
  if (click) {
    // The dialog opens on the project on screen; another section is a click.
    setTimeout(() => {
      const button = [...document.querySelectorAll<HTMLButtonElement>('.settings-nav-item')].find(
        (b) => b.textContent?.trim() === click,
      )
      button?.click()
    }, 150)
  }
}

export default defineGroup('Settings', [
  {
    id: 'project',
    title: 'A project',
    note: 'Opens on the project on screen. Setup steps and its own "Open in" targets are edited in place.',
    component: SettingsDialog,
    props: { open: true },
    setup: () => open('app'),
  },
  {
    id: 'empty-project',
    title: 'A project with nothing set up',
    component: SettingsDialog,
    props: { open: true },
    setup: () => open('docs'),
  },
  {
    id: 'committed',
    title: 'A committed .parallelist.json',
    note: 'Whatever the file sets wins, so the fields it covers say so and their entries are dimmed.',
    component: SettingsDialog,
    props: { open: true },
    setup: () =>
      open('app', {
        app: {
          file: '.parallelist.json',
          overrides: {
            baseBranch: 'staging',
            pathTemplate: null,
            setupSteps: [steps[3]],
            openTargets: null,
          },
          error: null,
        },
      }),
  },
  {
    id: 'invalid',
    title: 'A .parallelist.json that is not valid',
    component: SettingsDialog,
    props: { open: true },
    setup: () =>
      open('app', {
        app: {
          file: '.parallelist.json',
          overrides: { baseBranch: null, pathTemplate: null, setupSteps: null, openTargets: null },
          error: 'invalid type: integer `3`, expected a string at line 1 column 18',
        },
      }),
  },
  {
    id: 'general',
    title: 'General',
    component: SettingsDialog,
    props: { open: true },
    setup: () => open('app', {}, 'General'),
  },
  {
    id: 'targets',
    title: 'Open in',
    component: SettingsDialog,
    props: { open: true },
    setup: () => open('app', {}, 'Open in'),
  },
])
