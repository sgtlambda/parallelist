import {
  C,
  armDev,
  askPermission,
  banner,
  create,
  done,
  fill,
  renderOptions,
  runTurn,
  say,
  shellPrompt,
  tool,
  you,
  type Session,
} from './terminals'
import { world } from './world'

/**
 * The terminals already running when the page opens, in the states the marks
 * exist for: one blocked on an answer, two finished while nobody looked, two
 * still working, and a dev server that is merely running.
 */

const MIN = 60_000
const path = (project: string, branch: string) =>
  world[project].workspaces.find((w) => w.branch === branch)!.path

const heading = (question: string) => fill(`${C.b}${question}${C.r}`, ' ', ' ') + '\n'

function claude(project: string, branch: string, seed: Partial<Session>): Session {
  const dir = path(project, branch)
  const s = create(project, dir, { mode: 'claude', turns: 1, ...seed })
  s.backlog = banner(s.dir) + (seed.backlog ?? '')
  return s
}

export function seedTerminals() {
  const now = Date.now()

  // A dev server in the main workspace: running, saying nothing is wrong.
  const server = create('shop', path('shop', 'main'), {
    mode: 'dev',
    activeAt: now - 52 * MIN,
  })
  server.backlog =
    `${shellPrompt(server)}pnpm dev\n` +
    `\n  ${C.green}${C.b}VITE${C.r} v6.0.3  ${C.d}ready in${C.r} ${C.b}412 ms${C.r}\n\n` +
    `  ${C.green}➜${C.r}  ${C.b}Local${C.r}:   ${C.accent}http://localhost:5173/${C.r}\n` +
    `  ${C.d}➜  press h + enter to show help${C.r}\n`
  server.armed = () => armDev(server)

  // Blocked: asked something, and waiting for the answer.
  const flaky = claude('shop', '1234-e2e-flake', {
    backlog:
      you('checkout.spec.ts is flaky on ci again, roughly 1 in 5. never locally. find out why and fix it') +
      say("I'll start with what the test waits on before it clicks Pay.") +
      tool('Read', 'tests/e2e/checkout.spec.ts', 'Read 142 lines') +
      tool('Search', 'pattern: "postMessage"', 'Found 3 lines') +
      tool('Bash', 'gh run view 8841 --log-failed', 'Read 61 lines') +
      say(
        'The test clicks Pay as soon as the button renders, but the payment iframe posts `ready` about ' +
          '300ms later on the 2-CPU runners, so the click lands before the handler is attached. ' +
          "Three ways to wait, and the code can't pick one:",
      ),
    status: 'blocked',
    message: 'How should the test wait for the payment iframe?',
    question: 'How should the test wait for the payment iframe?',
    activeAt: now - 4 * MIN,
    waitingSince: now - 4 * MIN,
  })
  const waits = [
    "Wait for the iframe's `ready` message",
    'Poll until the Pay button is enabled',
    'Retry the test once on CI',
  ]
  flaky.ask = {
    options: waits,
    selected: 0,
    choose: (choice) =>
      runTurn(flaky, {
        verb: 'Cooking',
        steps: [
          {
            at: 1,
            name: 'Update',
            detail: choice === 2 ? 'playwright.config.ts' : 'tests/e2e/checkout.spec.ts',
            result: choice === 2 ? 'Updated with 1 addition' : 'Updated with 6 additions, 2 removals',
            message: choice === 2 ? 'Editing playwright.config.ts' : 'Editing checkout.spec.ts',
          },
          {
            at: 3,
            name: 'Bash',
            detail: 'pnpm exec playwright test checkout --repeat-each 20',
            result: choice === 2 ? '17 passed, 3 flaky' : '20 passed',
            message: 'Running checkout.spec.ts 20 times',
          },
        ],
        endAt: 5,
        finish: () =>
          done(
            flaky,
            choice === 0
              ? "The test now waits for the iframe's `ready` message before clicking Pay. " +
                  '20 runs in a row passed under a 2-CPU limit.'
              : choice === 1
                ? 'The test now polls until Pay is enabled, which the page does on `ready`. ' +
                  '20 runs in a row passed under a 2-CPU limit.'
                : 'CI now retries the test once (`retries: process.env.CI ? 1 : 0`). It still raced in 3 of ' +
                  '20 runs under a 2-CPU limit, so the flake is hidden rather than fixed.',
          ),
      }),
  }
  flaky.backlog += heading(flaky.question!) + renderOptions(flaky.ask)

  // Ready: finished, nothing typed there since, ending on a question.
  claude('shop', 'idem-keys', {
    backlog:
      you('make POST /orders idempotent. Idempotency-Key header, replay the stored response for 24h') +
      tool('Read', 'src/Http/Kernel.php', 'Read 68 lines') +
      tool('Write', 'src/Http/Middleware/IdempotencyKey.php', 'Wrote 71 lines') +
      tool('Update', 'routes/api.php', 'Updated with 3 additions') +
      tool('Bash', 'vendor/bin/phpunit --filter Idempotency', '6 passed') +
      say(
        'The middleware is in and the tests pass. A key seen again within 24 hours replays the stored ' +
          'response with its original status, and a key reused with a different body gets a 422.\n\n' +
          'One decision is yours: should the key be scoped per API token, or per shop?',
      ) +
      `${C.gray}❯${C.r} `,
    status: 'done',
    ready: true,
    question: 'should the key be scoped per API token, or per shop?',
    activeAt: now - 9 * MIN,
    waitingSince: now - 9 * MIN,
  })

  claude('api', 'retry-backoff', {
    backlog:
      you('retry failed webhooks with backoff, give up after a few tries') +
      tool('Read', 'src/webhooks/deliver.ts', 'Read 131 lines') +
      tool('Update', 'src/webhooks/deliver.ts', 'Updated with 38 additions') +
      tool('Write', 'test/backoff.test.ts', 'Wrote 52 lines') +
      tool('Bash', 'pnpm test webhooks', '9 passed') +
      say(
        'Retries now back off exponentially with jitter, capped at six attempts. A webhook that ' +
          'uses them all lands in the dead-letter table. The tests cover the schedule and the cap.',
      ) +
      `${C.gray}❯${C.r} `,
    status: 'done',
    ready: true,
    question:
      'A webhook that uses them all lands in the dead-letter table. The tests cover the schedule and the cap.',
    activeAt: now - 150_000,
    waitingSince: now - 150_000,
  })

  // Working: mid-turn. What each does next happens once the page is in view.
  const deadlock = claude('shop', 'reserve-lock', {
    backlog:
      you('two checkouts for the last unit of a sku deadlock in reserve(). take the locks in a stable order') +
      tool('Search', 'pattern: "lockForUpdate"', 'Found 4 lines') +
      tool('Read', 'src/Inventory/Reservation.php', 'Read 188 lines') +
      tool('Update', 'src/Inventory/Reservation.php', 'Updated with 9 additions, 3 removals'),
    status: 'working',
    message: 'Editing src/Inventory/Reservation.php',
    activeAt: now - 72_000,
  })
  seedSpinner(deadlock, 'Cooking', 72)
  deadlock.armed = () =>
    runTurn(deadlock, {
      verb: 'Cooking',
      elapsed: 72,
      steps: [
        {
          at: 80,
          name: 'Read',
          detail: 'tests/Inventory/ReservationTest.php',
          result: 'Read 96 lines',
          message: 'Reading tests/Inventory/ReservationTest.php',
        },
      ],
      endAt: 94,
      finish: () =>
        askPermission(deadlock, 'vendor/bin/phpunit --filter Reservation', 'Run the reservation tests'),
    })

  const layers = claude('design-system', 'css-layers', {
    backlog:
      you("put every component's styles in @layer components so app overrides win without !important") +
      tool('Search', 'pattern: "<style scoped>"', 'Found 31 files') +
      tool('Update', 'packages/button/src/Button.vue', 'Updated with 2 additions') +
      tool('Update', 'packages/dialog/src/Dialog.vue', 'Updated with 2 additions'),
    status: 'working',
    message: 'Updating snapshots for 31 stories',
    activeAt: now - 43_000,
  })
  seedSpinner(layers, 'Pondering', 43)
  layers.armed = () =>
    runTurn(layers, {
      verb: 'Pondering',
      elapsed: 43,
      steps: [],
      endAt: 53,
      finish: () =>
        done(
          layers,
          'All 31 components are in `@layer components`, and the snapshots are updated.\n\n' +
            'Should the tokens get a layer of their own (`@layer tokens, components`)?',
        ),
    })
}

/** The spinner line a turn `seconds` in would have drawn, so a terminal
 *  opened before the demo starts moving already shows one. */
function seedSpinner(s: Session, verb: string, seconds: number) {
  s.spinner = `${C.orange}✻${C.r} ${verb}… ${C.d}(${seconds}s · esc to interrupt)${C.r}`
}
