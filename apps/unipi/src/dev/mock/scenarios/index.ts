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
import { archived, draft, sessionMenu } from './session-actions';
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
import { slots, slotsDialogs, slotsOverlay, slotsTray } from './slots';
import { composerPaste, composerSlash, composerStates } from './composer';
import { composerAttachments, composerNarrow, imageEditorEllipse } from './composer-attachments';
import { editorConflict, editorLsp, filesPreview, filesSearch, filesTree } from './files';
import { cmdkEmpty, cmdkFiles, findBar, shortcutsList } from './command';
import { settingsExtensions, settingsForm, settingsFormAdvanced, settingsFormAgent, settingsFormList, settingsNav, settingsSafeMode, settingsSearch } from './settings';
import { needsYou, toastsScenario, unread } from './notify';
import { cronBadge, cronForm, cronList } from './cron';
import { rendererCards, toolsGroups, toolsMix } from './tools';
import { tabBtw, tabBtwPhone, tabDiff, tabDiffPhone, tabDiffSplit, tabSubagent, tabSubagentPhone, tabTree, tabTreePhone } from './tabs';
import { turnFiles, turnFilesConflict, turnFilesUndo } from './turn-files';
import { emptyChat, motionExits } from './empty-chat';
import { u12Fallback, u12Native, u12Nav, u12OwnerOff, u12Phone, u12PhoneOff } from './reserved';
import { composerCapsule, modeChip, modelCycleSet, modelDraftFusion, modelFusion, modelPicker, thinkingChip } from './model';
import {
  capAgents, capAgentsPanel, capImport, capMcp, capMcpNoChat, capMcpPanel, capMemory, capMemoryPanel, capPalette, capSkills, capSkillsPanel,
  providers, providersNoChat,
} from './capabilities';

export const SCENARIOS: Record<string, MockScenario> = {
  empty,
  'turn-files': turnFiles,
  'turn-files-undo': turnFilesUndo,
  'turn-files-conflict': turnFilesConflict,
  'u12-native': u12Native,
  'u12-fallback': u12Fallback,
  'u12-owner-off': u12OwnerOff,
  'u12-nav': u12Nav,
  'u12-phone': u12Phone,
  'u12-phone-off': u12PhoneOff,
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
  'session-menu': sessionMenu,
  archived,
  draft,
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
  'tab-diff': tabDiff,
  'tab-diff-split': tabDiffSplit,
  'tab-tree': tabTree,
  'tab-btw': tabBtw,
  'tab-subagent': tabSubagent,
  'tab-diff-phone': tabDiffPhone,
  'tab-tree-phone': tabTreePhone,
  'tab-btw-phone': tabBtwPhone,
  'tab-subagent-phone': tabSubagentPhone,
  'empty-chat': emptyChat,
  'motion-exits': motionExits,
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
  'slots-overlay': slotsOverlay,
  'slots-tray': slotsTray,
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
  'composer-attachments': composerAttachments,
  'composer-narrow': composerNarrow,
  'image-editor-ellipse': imageEditorEllipse,
  'settings-nav': settingsNav,
  'settings-search': settingsSearch,
  'settings-extensions': settingsExtensions,
  'settings-safe-mode': settingsSafeMode,
  'settings-form': settingsForm,
  'settings-form-list': settingsFormList,
  'settings-form-advanced': settingsFormAdvanced,
  'settings-form-agent': settingsFormAgent,
  'model-picker': modelPicker,
  'model-cycle-set': modelCycleSet,
  'model-fusion': modelFusion,
  'model-draft-fusion': modelDraftFusion,
  'thinking-chip': thinkingChip,
  'composer-capsule': composerCapsule,
  'mode-chip': modeChip,
  'cap-mcp': capMcp,
  'cap-mcp-nochat': capMcpNoChat,
  'cap-mcp-panel': capMcpPanel,
  'cap-memory': capMemory,
  'cap-memory-panel': capMemoryPanel,
  'cap-skills': capSkills,
  'cap-skills-panel': capSkillsPanel,
  'cap-agents': capAgents,
  'cap-agents-panel': capAgentsPanel,
  'cap-import': capImport,
  'cap-palette': capPalette,
  providers,
  'providers-nochat': providersNoChat,
};
