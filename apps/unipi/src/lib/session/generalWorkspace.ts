import { isHomeCwd } from './selectors';

/** Sessions in synthetic UUID/id folders or known scratch/test locations are not real projects. */
export function isGeneralWorkspaceCwd(cwd?: string | null): boolean {
  if (isHomeCwd(cwd) || !cwd) return true;
  const parts = cwd.replace(/[\\/]+$/, '').split(/[\\/]/).filter(Boolean);
  const folder = parts.at(-1) ?? '';
  if (/^(?:[0-9a-f]{8,}|[0-9a-f]{8}-[0-9a-f-]{27,})$/i.test(folder)) return true;
  // Only the system temp root folds (owner, 2026-11-12): a `tmp` folder inside a real project stays a project.
  if (/^\/tmp(?:\/|$)/.test(cwd)) return true;
  return parts.some((part) => /^\.od-projects$/i.test(part) || /^pytest-/i.test(part));
}
