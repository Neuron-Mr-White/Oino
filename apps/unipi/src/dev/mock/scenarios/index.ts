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
import { timelineMany } from './timeline';
import { bubbleFold, turnActions, turnNavigator } from './turn';
import { modalContext } from './modal-context';
import { modalInfo } from './modal-info';
import { modalLimits } from './modal-limits';
import { modalRunning } from './modal-running';
import { boardDashboard, boardLanes } from './board';
import { gitHistory, gitLastTurn, gitPane, gitPaneMany } from './git';
import { crashBoundary, dialogApproval, dialogDock, errorRow, takenOver } from './dock';
import { filesPreview, filesSearch, filesTree } from './files';

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
  'timeline-many': timelineMany,
  'turn-actions': turnActions,
  'turn-navigator': turnNavigator,
  'bubble-fold': bubbleFold,
  'modal-context': modalContext,
  'modal-running': modalRunning,
  'modal-info': modalInfo,
  'modal-limits': modalLimits,
  'board-lanes': boardLanes,
  'board-dashboard': boardDashboard,
  'git-pane': gitPane,
  'git-last-turn': gitLastTurn,
  'git-history': gitHistory,
  'git-pane-many': gitPaneMany,
  'files-tree': filesTree,
  'files-search': filesSearch,
  'files-preview': filesPreview,
  'dialog-dock': dialogDock,
  'dialog-approval': dialogApproval,
  'error-row': errorRow,
  'taken-over': takenOver,
  'crash-boundary': crashBoundary,
};
