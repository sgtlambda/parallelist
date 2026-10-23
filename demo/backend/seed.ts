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
  const header = claude('shop', '1234-fix-header', {
    backlog:
      you('The sticky header overlaps the cart drawer on mobile. Fix it.') +
      say("I'll start with how the header and the drawer are stacked.") +
      tool('Search', 'pattern: "z-index"', 'Found 7 lines') +
      tool('Read', 'src/components/AppHeader.vue', 'Read 96 lines') +
      say(
        "The header sits at `z-index: 50` and the drawer at `40`, so below 768px the header wins. " +
          "One thing the code can't tell me:",
      ),
    status: 'blocked',
    message: 'Which breakpoint should the header stop being sticky at?',
    question: 'Which breakpoint should the header stop being sticky at?',
    activeAt: now - 4 * MIN,
    waitingSince: now - 4 * MIN,
  })
  const breakpoints = [
    "640px, the drawer's breakpoint",
    '768px, the one-column layout',
    'Raise the drawer instead',
  ]
  header.ask = {
    options: breakpoints,
    selected: 0,
    choose: (choice) =>
      runTurn(header, {
        verb: 'Cooking',
        steps: [
          {
            at: 1,
            name: 'Update',
            detail: 'src/components/CartDrawer.vue',
            result: 'Updated with 1 addition, 1 removal',
            message: 'Editing CartDrawer.vue',
          },
          {
            at: 3,
            name: 'Bash',
            detail: 'pnpm test header',
            result: '8 passed',
            message: 'Running pnpm test header',
          },
        ],
        endAt: 5,
        finish: () =>
          done(
            header,
            choice === 2
              ? 'The drawer now sits above the header (`z-index: 60`), so the header can stay sticky. ' +
                  "I checked it at 375px and 1024px, and nothing else in src/components stacks above 50."
              : `The header now stops being sticky at ${choice === 0 ? '640' : '768'}px. ` +
                  'I checked it at 375px and 1024px.',
          ),
      }),
  }
  header.backlog += heading(header.question!) + renderOptions(header.ask)

  // Ready: finished while nobody was looking, ending on a question.
  claude('shop', 'feature/sso-login', {
    backlog:
      you('Add the SAML callback handler and wire it into the login route.') +
      tool('Read', 'src/auth/login.ts', 'Read 74 lines') +
      tool('Write', 'src/auth/saml.ts', 'Wrote 64 lines') +
      tool('Update', 'src/routes/login.ts', 'Updated with 9 additions') +
      tool('Bash', 'pnpm test auth', '12 passed') +
      say(
        'Both pieces are in and the auth tests pass. The callback checks the assertion signature and ' +
          'the audience, then hands the session to the existing login flow.\n\n' +
          'One decision is yours: should it also accept the legacy `/saml/acs` path, or only the new ' +
          '`/auth/saml/callback`?',
      ) +
      `${C.gray}❯${C.r} `,
    status: 'done',
    ready: true,
    question:
      'should it also accept the legacy `/saml/acs` path, or only the new `/auth/saml/callback`?',
    activeAt: now - 9 * MIN,
    waitingSince: now - 9 * MIN,
  })

  claude('api', 'feature/webhooks-retry', {
    backlog:
      you('Retry failed webhooks with backoff, and give up after a few tries.') +
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
  const invoice = claude('shop', 'fix/invoice-rounding', {
    backlog:
      you('Invoice totals are off by a cent when a line has three decimals. Round half-even.') +
      tool('Search', 'pattern: "round("', 'Found 11 lines') +
      tool('Read', 'src/Billing/Invoice.php', 'Read 212 lines') +
      tool('Update', 'src/Billing/Invoice.php', 'Updated with 7 additions'),
    status: 'working',
    message: 'Editing src/Billing/Invoice.php',
    activeAt: now - 72_000,
  })
  seedSpinner(invoice, 'Cooking', 72)
  invoice.armed = () =>
    runTurn(invoice, {
      verb: 'Cooking',
      elapsed: 72,
      steps: [
        {
          at: 80,
          name: 'Read',
          detail: 'tests/InvoiceTest.php',
          result: 'Read 96 lines',
          message: 'Reading tests/InvoiceTest.php',
        },
      ],
      endAt: 94,
      finish: () =>
        askPermission(invoice, 'pnpm test --filter billing', 'Run the billing tests'),
    })

  const buttons = claude('design-system', 'feature/button-variants', {
    backlog:
      you('Add ghost and link variants to Button, with stories.') +
      tool('Read', 'packages/button/src/Button.vue', 'Read 88 lines') +
      tool('Update', 'packages/button/src/Button.vue', 'Updated with 24 additions') +
      tool('Write', 'stories/Button.stories.ts', 'Wrote 41 lines'),
    status: 'working',
    message: 'Updating snapshots for 14 stories',
    activeAt: now - 43_000,
  })
  seedSpinner(buttons, 'Pondering', 43)
  buttons.armed = () =>
    runTurn(buttons, {
      verb: 'Pondering',
      elapsed: 43,
      steps: [],
      endAt: 53,
      finish: () =>
        done(
          buttons,
          'Both variants are in, with stories, and the 14 snapshots are updated.\n\n' +
            'Should I add the `ghost` variant to the docs page as well?',
        ),
    })
}

/** The spinner line a turn `seconds` in would have drawn, so a terminal
 *  opened before the demo starts moving already shows one. */
function seedSpinner(s: Session, verb: string, seconds: number) {
  s.spinner = `${C.orange}✻${C.r} ${verb}… ${C.d}(${seconds}s · esc to interrupt)${C.r}`
}
