/**
 * A host call that never answers must not leave a screen on "Checking…"
 * forever (fix3-polish): after `HOST_STALL_MS` the caller gets the fallback
 * (an offline/timeout result) and shows the calm offline state instead.
 */
export const HOST_STALL_MS = 12_000;

export function within<T>(work: Promise<T>, onStall: () => T, ms: number = HOST_STALL_MS): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => resolve(onStall()), ms);
    work.then(
      (value) => (clearTimeout(timer), resolve(value)),
      (error) => (clearTimeout(timer), reject(error)),
    );
  });
}
