import { backend } from '../backend'

type Event<T> = { event: string; id: number; payload: T }

export function listen<T>(event: string, handler: (e: Event<T>) => void): Promise<() => void> {
  return Promise.resolve(backend.listen(event, handler as (e: Event<unknown>) => void))
}
