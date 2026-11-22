/**
 * App navigation for non-component code (WP-A25): the composer's built-in
 * routing, the Settings "Manage…" actions and the import dialog all need to
 * move the router without a component scope. The shell registers the real
 * `useNavigate()` once (`AppShell`); everything else falls back to the hash the
 * router already listens to (the app is a `HashRouter`), so a test or a
 * headless call still lands on the right route.
 */
export type NavigateFn = (href: string, opts?: { replace?: boolean }) => void;

let navigate: NavigateFn | undefined;

/** Installs the router's navigate (returns the uninstall). */
export function setAppNavigate(fn: NavigateFn | undefined): () => void {
  const prev = navigate;
  navigate = fn;
  return () => {
    if (navigate === fn) navigate = prev;
  };
}

/** Navigates to an app route (`/settings/about`). */
export function appNavigate(href: string, opts?: { replace?: boolean }): void {
  if (navigate) {
    navigate(href, opts);
    return;
  }
  if (typeof location === 'undefined') return;
  location.hash = `#${href}`;
}
