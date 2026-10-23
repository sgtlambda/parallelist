import type { Group, Scenario } from './define'

/** Every scenario file's group, found by glob. Kept apart from `define.ts`:
 *  the files imported here import that, and the other way round is a cycle. */
const slug = (text: string) => text.toLowerCase().replace(/[^a-z0-9]+/g, '-')

export const groups: Group[] = Object.values(
  import.meta.glob<{ default: Group }>('./scenarios/*.ts', { eager: true }),
)
  .map((m) => m.default)
  .sort((a, b) => a.name.localeCompare(b.name))

export const idOf = (group: Group, scenario: Scenario) => `${slug(group.name)}/${scenario.id}`

export function findScenario(id: string): Scenario | null {
  for (const group of groups) {
    for (const scenario of group.scenarios) if (idOf(group, scenario) === id) return scenario
  }
  return null
}
