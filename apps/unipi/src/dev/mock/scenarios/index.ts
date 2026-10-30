/** Capture/test scenarios by name (`?scenario=<name>`). Later WPs append theirs. */
import type { MockScenario } from '../host';
import { empty } from './empty';
import { noHost } from './no-host';
import { offline } from './offline';
import { streamBasic } from './stream-basic';
import { streamReconnect } from './stream-reconnect';
import { streamSteer } from './stream-steer';
import { queueHeld, queuePanel } from './queue';
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
import { capsuleScenario, statusLhGraph, statusRunning, statusTodos } from './status';
import { crashBoundary, dialogApproval, dialogDock, errorRow, takenOver } from './dock';
import { slots, slotsDialogs } from './slots';
import { composerPaste, composerSlash, composerStates } from './composer';
import { editorConflict, editorLsp, filesPreview, filesSearch, filesTree } from './files';
import { cmdkEmpty, cmdkFiles, findBar, shortcutsList } from './command';
import { settingsExtensions, settingsNav, settingsSafeMode, settingsSearch } from './settings';
import { needsYou, toastsScenario, unread } from './notify';
import { cronBadge, cronForm, cronList } from './cron';
import { rendererCards, toolsGroups, toolsMix } from './tools';

export const SCENARIOS: Record<string, MockScenario> = {
  empty,
  'no-host': noHost,
  offline,
  'stream-basic': streamBasic,
  'stream-steer': streamSteer,
  queue: queuePanel,
  'queue-held': queueHeld,
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
  'status-lh-graph': statusLhGraph,
  'status-todos': statusTodos,
  'status-running': statusRunning,
  capsule: capsuleScenario,
  'files-tree': filesTree,
  'tools-mix': toolsMix,
  'tools-groups': toolsGroups,
  'renderer-cards': rendererCards,
  'files-search': filesSearch,
  'files-preview': filesPreview,
  'editor-lsp': editorLsp,
  'editor-conflict': editorConflict,
  'cron-list': cronList,
  'cron-form': cronForm,
  'cron-badge': cronBadge,
  'dialog-dock': dialogDock,
  'dialog-approval': dialogApproval,
  'error-row': errorRow,
  'taken-over': takenOver,
  'crash-boundary': crashBoundary,
  slots,
  'slots-dialogs': slotsDialogs,
  'needs-you': needsYou,
  toasts: toastsScenario,
  unread,
  'cmdk-empty': cmdkEmpty,
  'cmdk-files': cmdkFiles,
  'find-bar': findBar,
  'shortcuts-list': shortcutsList,
  'composer-states': composerStates,
  'composer-paste': composerPaste,
  'composer-slash': composerSlash,
  'settings-nav': settingsNav,
  'settings-search': settingsSearch,
  'settings-extensions': settingsExtensions,
  'settings-safe-mode': settingsSafeMode,
};
