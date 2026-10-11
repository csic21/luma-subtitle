/** In-memory fixture event bus. No Tauri IPC or user-system access. */
const listeners = new Set<(event: { payload: unknown }) => void>();
export type UnlistenFn = () => void;
export async function listen<T>(_name: string, callback: (event: { payload: T }) => void): Promise<UnlistenFn> {
  const handler = callback as (event: { payload: unknown }) => void;
  listeners.add(handler);
  return () => { listeners.delete(handler); };
}
export function emitFixtureProgress(payload: unknown) { for (const callback of listeners) callback({ payload }); }
