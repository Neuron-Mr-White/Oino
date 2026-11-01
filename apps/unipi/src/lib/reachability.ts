/**
 * Unipi feature reachability (WP-A25, DIRECTION §4.2.1 rule 1 + zcode-audit
 * "Unipi feature reachability"): the audit table as **data**, so the gap
 * cannot reopen silently.
 *
 * Every unipi extension row of the audit names the app surface(s) that make
 * its features reachable, and every surface resolves to code this app ships —
 * `reachability.test.ts` walks the table and fails on any row without a
 * surface, and on any surface whose evidence does not exist.
 *
 * Surface id grammar (what the acceptance row lists):
 *
 *   settings:<section>     a Settings page (`screens/settings/catalog.ts`)
 *   tab:<kind>             a side-pane / bottom-panel tab kind (`lib/panes/kinds.ts`)
 *   route:<path>           an app route (`main.tsx`)
 *   chat:<id>              a conversation ⋯ row (`components/shell/ConversationHeader.tsx`)
 *   slot:<placement>       a UI v2 slot host (`ui/slots/*`, U10)
 *   app:<id>               an app action/surface in `APP_SURFACES` below
 *
 * The pi built-ins row is per command: each has a `builtinPlan` whose surface
 * must equal the one listed here (G-15 — no built-in falls back to a toast).
 */
import { CHAT_ACTIONS } from '../components/shell/ConversationHeader';
import { BUILTIN_KINDS } from './panes/kinds';
import { builtinPlan } from './composer/builtins';
import { SETTINGS_SECTIONS } from '../screens/settings/catalog';

/** App routes (`main.tsx` `Root()`), plus the settings sub-route pattern. */
export const ROUTES = ['/', '/s/:host/:session', '/new/:host', '/chat', '/board', '/cron', '/x/:entryId', '/settings', '/settings/:section', '/files', '/files/edit', '/preview'];

/** UI v2 slot hosts (U10): the placement → the module that renders it. */
export const SLOT_SURFACES: Array<{ id: string; title: string; evidence: string }> = [
  { id: 'dialog', title: 'Dialogs in the dock', evidence: '../ui/dialogs/DialogCard.tsx#DialogCard' },
  { id: 'overlay', title: 'Overlay panels (modal)', evidence: '../ui/slots/SlotModal.tsx#SlotModal' },
  { id: 'side', title: 'Side-pane tabs', evidence: '../ui/slots/paneTabs.tsx#ExtPaneTab' },
  { id: 'bottom', title: 'Bottom-panel tabs', evidence: '../ui/slots/paneTabs.tsx#ExtPaneTab' },
  { id: 'tray', title: 'Composer tray (editorReplace)', evidence: '../ui/slots/ComposerTray.tsx#ComposerTray' },
  { id: 'fullscreen', title: 'Sidebar pages', evidence: '../ui/slots/pages.tsx#ExtensionPage' },
  { id: 'widget', title: 'Above the composer', evidence: '../ui/slots/WidgetStack.tsx#WidgetStack' },
  { id: 'status', title: 'Status strip', evidence: '../ui/slots/statusFeed.ts#feedStatusStrip' },
  { id: 'turn-footer', title: 'Turn footer', evidence: '../ui/slots/TurnFooters.tsx#TurnFooters' },
];

/**
 * App surfaces the audit rows point at that are not a page, tab, route, ⋯
 * row or slot. `evidence` is `module#export` (a file with no `#` = the file
 * itself); the test imports it so a renamed/moved surface fails the gate.
 */
export const APP_SURFACES: Array<{ id: string; title: string; evidence: string }> = [
  { id: 'session-modal', title: 'Session modal (Context · Running · Info)', evidence: '../components/session/SessionModal.tsx#openSessionModal' },
  { id: 'model-picker', title: 'Model picker (single + Fusion)', evidence: '../components/model/ModelPicker.tsx#ModelPicker' },
  { id: 'model-picker-cycle-set', title: 'Model picker, Cycle set', evidence: '../components/model/ModelPicker.tsx#ModelPickerBody' },
  { id: 'status-pane', title: 'Status pane (Git · LH · todos)', evidence: '../components/status/StatusPane.tsx#openStatusSection' },
  { id: 'composer-permission', title: 'Composer permission / plan control', evidence: '../components/composer/PermissionControl.tsx#PermissionControl' },
  { id: 'keyboard-shortcuts', title: 'Ctrl+/ keyboard list', evidence: '../components/shell/ShortcutsDialog.tsx#setShortcutsOpen' },
  { id: 'import-session', title: 'Import session… dialog', evidence: '../components/command/ImportSession.tsx#openImportSession' },
  { id: 'quit', title: 'Quit (desktop lifecycle)', evidence: '../components/LifecycleUi.tsx#QuitRunner' },
  { id: 'notice-share-dropped', title: 'Share: dropped, Export pointed at', evidence: '../lib/composer/builtins.ts#BUILTIN_ROUTES' },
  { id: 'notice-trust-dropped', title: 'Trust: dropped (host decides)', evidence: '../lib/composer/builtins.ts#BUILTIN_ROUTES' },
];

/** One audit row: an extension and where its user-facing features live. */
export type ReachabilityRow = {
  /** Audit row id (the extension's package name, `pi-builtins` for the last row). */
  id: string;
  /** The audit table's "User-facing features" summary (short). */
  feature: string;
  /** The G gap(s) this row closes with A25 (empty for rows closed by an earlier WP). */
  gaps?: string[];
  /** Surface ids; every one must resolve. */
  surfaces: string[];
};

/** The audit's 23 rows, in its order. */
export const REACHABILITY: ReachabilityRow[] = [
  {
    id: 'ask-user',
    feature: 'ask_user form dialog, end-turn launcher, settings',
    surfaces: ['slot:dialog', 'settings:extensions'],
  },
  {
    id: 'background-tasks',
    feature: 'bg tools, /bg, Ctrl+Alt+C clear, work-tray tab, settings',
    surfaces: ['app:session-modal', 'settings:extensions'],
  },
  { id: 'btw', feature: '/btw side-chat panel', surfaces: ['tab:btw', 'settings:extensions'] },
  { id: 'compactor', feature: '/compact*, stats/doctor panels, settings', surfaces: ['slot:overlay', 'settings:extensions'] },
  { id: 'core', feature: 'work tray, hint line, decision-model settings', surfaces: ['slot:tray', 'app:session-modal', 'settings:extensions'] },
  { id: 'cron', feature: '/cron + add/edit/remove, cron tools, settings', surfaces: ['route:/cron', 'settings:extensions'] },
  { id: 'dream', feature: '/dream status/tray/report/run/stop, settings', surfaces: ['slot:overlay', 'settings:extensions'] },
  { id: 'fusion', feature: 'sidekick tool, /fusion picker, presets, wake widget', surfaces: ['app:model-picker', 'settings:extensions'] },
  { id: 'info-screen', feature: '/info pages (Session / Usage / Tools / Skills / Modules)', surfaces: ['app:session-modal', 'settings:extensions'] },
  { id: 'kanboard', feature: '/kanboard*, reminders, in-chat board, side board', surfaces: ['route:/board', 'settings:extensions'] },
  { id: 'long-horizon', feature: '/goal /ralph /swarm /graph, progress, mode slots', surfaces: ['app:status-pane', 'settings:extensions'] },
  {
    id: 'mcp',
    feature: '/mcp-status, server list, catalog, JSON edit, reload',
    gaps: ['G-8'],
    surfaces: ['settings:mcp', 'slot:overlay'],
  },
  {
    id: 'memory',
    feature: 'memory_* tools, /memory*, browse/search/forget/consolidate',
    gaps: ['G-9'],
    surfaces: ['settings:memory', 'slot:overlay'],
  },
  { id: 'notify', feature: 'notify_user, event matrix, setup wizards', surfaces: ['settings:notifications', 'settings:extensions'] },
  {
    id: 'skill-registry',
    feature: 'exposure judged/all/off, pins, vault, /skills hub, editor',
    gaps: ['G-10'],
    surfaces: ['settings:skills', 'slot:overlay'],
  },
  {
    id: 'subagents',
    feature: 'run/read_subagent, /subagents tray, /agents profiles, all-to-background',
    gaps: ['G-11'],
    surfaces: ['settings:subagents', 'slot:overlay', 'app:session-modal'],
  },
  { id: 'updater', feature: 'Oino release check overlay, /readme, settings', surfaces: ['settings:about'] },
  { id: 'utility', feature: '/continue /doctor /cleanup, auto-rename, images, settings', surfaces: ['slot:overlay', 'settings:extensions'] },
  { id: 'watchdog', feature: 'watchdog actions, warnings, per-session caps', surfaces: ['app:session-modal', 'settings:extensions'] },
  { id: 'web-api', feature: 'web_search / read / summarize, provider keys', surfaces: ['settings:extensions', 'settings:providers'] },
  { id: 'workflow', feature: 'permission gate, approval dialog, /permission, /plan', surfaces: ['app:composer-permission', 'slot:dialog', 'settings:extensions'] },
];

/** The pi built-ins (G-15), one row per command: the audit table's last row. */
export const BUILTIN_REACHABILITY: ReachabilityRow[] = [
  { id: 'pi:export', feature: '/export', gaps: ['G-15'], surfaces: ['chat:export'] },
  { id: 'pi:import', feature: '/import', gaps: ['G-15'], surfaces: ['app:import-session'] },
  { id: 'pi:clone', feature: '/clone', gaps: ['G-15'], surfaces: ['chat:duplicate'] },
  { id: 'pi:share', feature: '/share', gaps: ['G-15'], surfaces: ['app:notice-share-dropped'] },
  { id: 'pi:scoped-models', feature: '/scoped-models', gaps: ['G-15'], surfaces: ['app:model-picker-cycle-set'] },
  { id: 'pi:login', feature: '/login', gaps: ['G-15'], surfaces: ['settings:providers'] },
  { id: 'pi:logout', feature: '/logout', gaps: ['G-15'], surfaces: ['settings:providers'] },
  { id: 'pi:settings', feature: '/settings', gaps: ['G-15'], surfaces: ['route:/settings'] },
  { id: 'pi:changelog', feature: '/changelog', gaps: ['G-15'], surfaces: ['settings:about'] },
  { id: 'pi:hotkeys', feature: '/hotkeys', gaps: ['G-15'], surfaces: ['app:keyboard-shortcuts'] },
  { id: 'pi:trust', feature: '/trust', gaps: ['G-15'], surfaces: ['app:notice-trust-dropped'] },
  { id: 'pi:bug', feature: '/bug', gaps: ['G-15'], surfaces: ['settings:about'] },
  { id: 'pi:quit', feature: '/quit', gaps: ['G-15'], surfaces: ['app:quit'] },
];

/** Every row the gate walks: the audit's extension rows + the built-ins. */
export function reachabilityRows(): ReachabilityRow[] {
  return [...REACHABILITY, ...BUILTIN_REACHABILITY];
}

/** True when `ref` names a surface this build has (see the module header for the grammar). */
export function surfaceResolves(ref: string): boolean {
  const at = ref.indexOf(':');
  if (at < 0) return false;
  const kind = ref.slice(0, at);
  const id = ref.slice(at + 1);
  switch (kind) {
    case 'settings':
      return SETTINGS_SECTIONS.some((section) => section.id === id);
    case 'tab':
      return BUILTIN_KINDS.some((def) => def.kind === id);
    case 'route':
      return ROUTES.includes(id);
    case 'chat-action':
    case 'chat':
      return CHAT_ACTIONS.some((row) => row.id === id);
    case 'slot':
      return SLOT_SURFACES.some((slot) => slot.id === id);
    case 'app':
      return APP_SURFACES.some((surface) => surface.id === id);
    default:
      return false;
  }
}

/** Every surface id in the table that does not resolve (empty = the gate passes). */
export function unresolvedSurfaces(rows: readonly ReachabilityRow[] = reachabilityRows()): string[] {
  const bad = new Set<string>();
  for (const row of rows) for (const surface of row.surfaces) if (!surfaceResolves(surface)) bad.add(`${row.id} → ${surface}`);
  return [...bad];
}

/**
 * G-15 drift check: the built-in rows and the routing table must name the same
 * commands and the same surfaces. Returns a list of mismatches (empty = fine).
 */
export function builtinRouteMismatches(): string[] {
  const out: string[] = [];
  for (const row of BUILTIN_REACHABILITY) {
    const name = row.id.replace(/^pi:/, '');
    const plan = builtinPlan(name);
    if (!plan) {
      out.push(`${name}: no route`);
      continue;
    }
    if (!row.surfaces.includes(plan.surface)) out.push(`${name}: route opens ${plan.surface}, table says ${row.surfaces.join(' / ')}`);
  }
  return out;
}
