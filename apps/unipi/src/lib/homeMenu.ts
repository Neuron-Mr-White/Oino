/**
 * Home long-press menu per group (DESIGN §3.1, UNI-128 follow-up).
 * Pure so the "nothing here kills a real session" rule is testable:
 * every action is local to the app (resume, rename the label, detach
 * our connection, edit the recents list). There is deliberately no
 * host-side close/kill action.
 */
export type HomeGroup = 'ongoing' | 'recent';

export type HomeMenuAction = 'open' | 'rename' | 'close-connection' | 'remove-recent';

export type HomeMenuItem = {
  action: HomeMenuAction;
  label: string;
  tone?: 'danger';
};

export function homeMenuItems(group: HomeGroup, opts: { canRename: boolean }): HomeMenuItem[] {
  const items: HomeMenuItem[] = [{ action: 'open', label: group === 'ongoing' ? 'Open' : 'Reconnect' }];
  if (opts.canRename) items.push({ action: 'rename', label: 'Rename label' });
  if (group === 'ongoing') {
    // Detach only: the herdr workspace / tmux session keeps running.
    items.push({ action: 'close-connection', label: 'Close connection' });
  } else {
    items.push({ action: 'remove-recent', label: 'Remove from Recents', tone: 'danger' });
  }
  return items;
}
