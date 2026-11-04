/**
 * [sidebar-ui] "Dream" from the sidebar (project head menu, session row menu): the memory
 * extension's `dream run` for one project, through the same Settings background-command path as
 * Settings › Memory (`run.ts`), in a hidden command chat rooted at that folder. One notice line
 * says it started and, when it answers, what it said.
 */
import { showActionNotice } from '../timeline/actions';
import { runInBackground } from './run';

const folderName = (cwd: string | undefined) => cwd?.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || undefined;

export async function runDream(hostId: string, cwd: string | undefined, label?: string): Promise<void> {
  const where = label || folderName(cwd);
  showActionNotice(where ? `Dreaming in ${where}…` : 'Dreaming…');
  const outcome = await runInBackground('dream run', { hostId, ...(cwd ? { cwd } : {}) });
  if (outcome.kind === 'done') showActionNotice(outcome.replies[outcome.replies.length - 1]?.text.split('\n')[0] || 'Dream finished');
  else if (outcome.kind === 'quiet') showActionNotice('Dream is running — see the memory report when it finishes');
  else if (outcome.kind === 'missing') showActionNotice('Dream needs the memory extension — turn it on in Settings › Extensions');
  else showActionNotice('Couldn’t reach this PC to dream');
}
