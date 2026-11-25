import SettingsDialog from '../../components/SettingsDialog.vue'
import type { ProjectFile, ScreenSearchModel, ScreenSearchStatus } from '../../api'
import { screenStatus } from '../../screenSearch'
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

const MODELS: ScreenSearchModel[] = [
  { id: 'minilm-l6', label: 'Small', name: 'all-MiniLM-L6-v2', downloadMb: 90, memoryMb: 500 },
  { id: 'minilm-l12', label: 'Medium', name: 'all-MiniLM-L12-v2', downloadMb: 135, memoryMb: 550 },
  { id: 'mpnet-base', label: 'Large', name: 'all-mpnet-base-v2', downloadMb: 440, memoryMb: 1200 },
]

async function open(
  selected: string | null,
  files: Record<string, ProjectFile> = {},
  click?: string,
  screen?: { status: ScreenSearchStatus },
) {
  await boot({ config: { ...settingsConfig(selected), screenSearch: !!screen } })
  backend.handle({
    project_files: () => files,
    save_config: () => {},
    screen_search_models: () => MODELS,
  })
  if (screen) screenStatus.value = screen.status
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
    id: 'screen-search',
    title: 'Screen search, its model downloading',
    note: 'The model picked, and how far its download has got. A failure says why in red.',
    component: SettingsDialog,
    props: { open: true },
    setup: () => open('app', {}, 'Screen search', {
      status: { state: 'loading', model: 'minilm-l6', downloaded: 45_000_000, size: 90_000_000 },
    }),
  },
  {
    id: 'screen-search-failed',
    title: 'Screen search, its model failed',
    component: SettingsDialog,
    props: { open: true },
    setup: () =>
      open('app', {}, 'Screen search', {
        status: { state: 'failed', model: 'minilm-l6', error: 'Failed to retrieve model.onnx: network unreachable' },
      }),
  },
  {
    id: 'screen-search-off',
    title: 'Screen search, off',
    note: 'The model can be seen, greyed out, until the switch is on.',
    component: SettingsDialog,
    props: { open: true },
    setup: () => open('app', {}, 'Screen search'),
  },
  {
    id: 'targets',
    title: 'Open in',
    component: SettingsDialog,
    props: { open: true },
    setup: () => open('app', {}, 'Open in'),
  },
])
