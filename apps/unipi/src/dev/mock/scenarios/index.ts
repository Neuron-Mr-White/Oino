/** Capture/test scenarios by name (`?scenario=<name>`). Later WPs append theirs. */
import type { MockScenario } from '../host';
import { empty } from './empty';
import { noHost } from './no-host';
import { offline } from './offline';
import { streamBasic } from './stream-basic';
import { streamReconnect } from './stream-reconnect';
import { streamSteer } from './stream-steer';
import { shellDesktop, shellFullview, shellNarrow, shellSidepaneTabs } from './shell';
import { sidebarMany, sidebarStates } from './sidebar';
import { gitHistory, gitLastTurn, gitPane, gitPaneMany } from './git';
import { boardDashboard, boardLanes } from './board';

export const SCENARIOS: Record<string, MockScenario> = {
  empty,
  'no-host': noHost,
  offline,
  'stream-basic': streamBasic,
  'stream-steer': streamSteer,
  'stream-reconnect': streamReconnect,
  'shell-desktop': shellDesktop,
  'shell-narrow': shellNarrow,
  'shell-sidepane-tabs': shellSidepaneTabs,
  'shell-fullview': shellFullview,
  'sidebar-many': sidebarMany,
  'sidebar-states': sidebarStates,
  'git-pane': gitPane,
  'git-last-turn': gitLastTurn,
  'git-history': gitHistory,
  'git-pane-many': gitPaneMany,
  'board-lanes': boardLanes,
  'board-dashboard': boardDashboard,
};
