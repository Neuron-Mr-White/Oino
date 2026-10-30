/**
 * Pane tab kinds (app spec §3.8, WP-A3): one registry for the side pane and
 * the bottom panel. Each kind says how it looks in the strip, whether it is a
 * singleton, its scope (project or session), where it may open and how the
 * `+` launcher offers it.
 *
 * A3 declares the built-in kinds; the WP that builds a kind's view plugs its
 * component in with **one line** (append-only, risk R9):
 *
 *   setPaneComponent('files', FilesTab);           // A20
 *   registerPaneKind({ kind: 'ext', … });          // U10 (or setPaneComponent)
 *
 * A kind without a component is never offered by the launcher and a
 * persisted tab of it renders the "not available" state.
 */
import { createSignal, type Component } from 'solid-js';
import { Bot, FileDiff, FolderTree, GitBranch, Globe, Image, ListTree, MessageSquareText, Plug, Puzzle, SquareTerminal } from 'lucide-solid';
import { setPaneRules, type KindRule, type PaneArea, type PaneTab } from '../panes';

export type PaneTabProps = {
  tab: PaneTab;
  area: PaneArea;
  hostId: string;
  projectKey: string;
  sessionId?: string;
  /** Closes this tab. */
  close: () => void;
  /** Sets the strip title (e.g. a page title). */
  setTitle: (title: string | undefined) => void;
};

export type IconComponent = Component<{ size?: number; strokeWidth?: number; class?: string }>;

export type PaneKindDef = KindRule & {
  kind: string;
  icon: IconComponent;
  title: (params: Record<string, unknown>) => string;
  component?: Component<PaneTabProps>;
  /** Where it may open (default side). */
  areas?: PaneArea[];
  /** Offered by the `+` launcher (default: not offered). */
  launcher?: { label: string; order: number; needsSession?: boolean; keywords?: string };
  /** Reserved slot id it stands for (ui-protocol reserved routing), for the record. */
  reserved?: string;
  /** Runs before the store closes a tab of this kind (U10: `ext` sends the entry `$close`). */
  onUserClose?: (tab: PaneTab) => void;
};

const str = (v: unknown, fallback: string): string => (typeof v === 'string' && v ? v : fallback);
const leaf = (path: unknown, fallback: string): string => str(path, fallback).split('/').filter(Boolean).pop() ?? fallback;

/** The built-in kinds (§3.8 table). Components arrive with their WPs. */
export const BUILTIN_KINDS: PaneKindDef[] = [
  { kind: 'files', icon: FolderTree, title: () => 'Files', singleton: true, scope: 'project', reserved: 'host/files', launcher: { label: 'Files', order: 10, keywords: 'tree editor' } },
  { kind: 'git', icon: GitBranch, title: () => 'Git', singleton: true, scope: 'project', reserved: 'host/git', launcher: { label: 'Git', order: 20, keywords: 'changes commit status' } },
  { kind: 'tree', icon: ListTree, title: () => 'Tree', singleton: true, scope: 'session', launcher: { label: 'Tree', order: 30, needsSession: true, keywords: 'session branches fork' } },
  { kind: 'btw', icon: MessageSquareText, title: (p) => str(p.title, 'Side chat'), singleton: false, scope: 'session', launcher: { label: 'Side chat', order: 40, needsSession: true, keywords: 'btw aside question' } },
  { kind: 'diff', icon: FileDiff, title: (p) => leaf(p.path, 'Diff'), singleton: (p) => str(p.path, ''), scope: 'session' },
  { kind: 'preview', icon: Image, title: (p) => leaf(p.path, 'Preview'), singleton: (p) => str(p.path, ''), scope: 'project' },
  { kind: 'subagent', icon: Bot, title: (p) => str(p.name, 'Subagent'), singleton: (p) => str(p.agentId, ''), scope: 'session' },
  { kind: 'browser', icon: Globe, title: (p) => str(p.title, 'Browser'), singleton: false, scope: 'project', reserved: 'unipi.browser/browser', launcher: { label: 'Browser', order: 50 } },
  { kind: 'terminal', icon: SquareTerminal, title: (p) => str(p.title, 'Terminal'), singleton: false, scope: 'project', areas: ['bottom', 'side'], reserved: 'host/terminal', launcher: { label: 'Terminal', order: 60 } },
  { kind: 'ext', icon: Puzzle, title: (p) => str(p.title, 'Extension'), singleton: (p) => str(p.entryId, ''), scope: 'project', areas: ['side', 'bottom'] },
  // Z-104: a forwarded port of the host (the forward viewer until the browser tab lands).
  { kind: 'forward', icon: Plug, title: (p) => `localhost:${String(p.port ?? '')}`, singleton: (p) => `${String(p.hostId ?? '')}:${String(p.port ?? '')}`, scope: 'project' },
];

const [kinds, setKinds] = createSignal<Record<string, PaneKindDef>>(Object.fromEntries(BUILTIN_KINDS.map((k) => [k.kind, k])));

/** Adds or replaces a kind. Returns the unregister (restores the previous definition). */
export function registerPaneKind(def: PaneKindDef): () => void {
  const prev = kinds()[def.kind];
  setKinds((cur) => ({ ...cur, [def.kind]: def }));
  return () =>
    setKinds((cur) => {
      if (cur[def.kind] !== def) return cur;
      const next = { ...cur };
      if (prev) next[def.kind] = prev;
      else delete next[def.kind];
      return next;
    });
}

/** Plugs a view into a declared kind (one line per WP). Returns the unplug. */
export function setPaneComponent(kind: string, component: Component<PaneTabProps>): () => void {
  const def = kinds()[kind];
  if (!def) throw new Error(`unknown pane kind "${kind}"`);
  return registerPaneKind({ ...def, component });
}

export function paneKind(kind: string): PaneKindDef | undefined {
  return kinds()[kind];
}

export function paneKinds(): PaneKindDef[] {
  return Object.values(kinds());
}

/** The launcher's entries for an area: kinds with a view, in launcher order. */
export function launchableKinds(area: PaneArea, hasSession: boolean): PaneKindDef[] {
  return paneKinds()
    .filter((k) => k.launcher && k.component && (k.areas ?? ['side']).includes(area) && (hasSession || !k.launcher.needsSession))
    .sort((a, b) => a.launcher!.order - b.launcher!.order);
}

/** The strip title of a tab. */
export function tabTitle(tab: PaneTab): string {
  return tab.title ?? paneKind(tab.kind)?.title(tab.params ?? {}) ?? tab.kind;
}

setPaneRules((kind) => kinds()[kind]);

// The session tab views (`diff`, `tree`, `btw`, `subagent`, WP-A10) register
// themselves when their module loads; `main.tsx` imports
// `components/panes/tabs` once, before the first render (the spec's shared
// append-only registration point). This module must not import them: the tab
// modules reach the side pane and the shown workspace, which import this
// registry.
