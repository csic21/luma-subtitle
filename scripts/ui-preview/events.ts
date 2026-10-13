/** Isolated in-memory event bus. Never calls native IPC. */
export type UnlistenFn = () => void;
export type Event<T> = { event: string; id: number; payload: T };
export type EventCallback<T> = (event: Event<T>) => void;
const listeners = new Map<string, Set<EventCallback<unknown>>>();
let sequence = 0;
export async function listen<T>(name: string, callback: EventCallback<T>): Promise<UnlistenFn> {
  const handlers = listeners.get(name) ?? new Set();
  listeners.set(name, handlers);
  const handler = callback as EventCallback<unknown>;
  handlers.add(handler);
  return () => { handlers.delete(handler); };
}
export function emitFixture(name: string, payload: unknown) {
  const event = { event: name, id: ++sequence, payload };
  for (const handler of listeners.get(name) ?? []) handler(event);
}
