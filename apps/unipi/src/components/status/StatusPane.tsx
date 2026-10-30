/**
 * The status pane (RULINGS Z-95, §3.10): the `git` side-pane tab (reserved
 * `host/git`) — the Git section (WP-A15) stacked over the WP-A16 status
 * sections: Long-horizon, Plan & todos (Z-43) and the compact Running
 * summary that links to the Session modal. On the phone the pane is the
 * full-screen layer of §3.9 (`StatusPaneLayer`, back button).
 *
 * One Git UI (Z-100): everything opens this pane — the `+` launcher, the
 * Files branch chip, the chat ⋯ Git entry, Cmd/Ctrl+Alt+B (the side-pane
 * toggle) and the collapsed capsule (A16). `openStatusSection` is the app's
 * entry point for "open/focus the status pane (at a section)" — the palette
 * command, the ModeChip's routing and the reserved
 * `unipi.long-horizon/visualize` entry all funnel here.
 */
import { createEffect, createSignal, onCleanup, onMount, Show } from 'solid-js';
import { ArrowLeft } from 'lucide-solid';
import type { PaneTabProps } from '../../lib/panes/kinds';
import { setPaneComponent } from '../../lib/panes/kinds';
import { pushBack } from '../../lib/back';
import { desktopShell, layout } from '../../lib/layout';
import { panes } from '../../lib/panes';
import { currentWorkspace } from '../shell/Workspace';
import { closeGit, gitParams, gitTarget, openGit, type GitSource } from '../../lib/git';
import '../git/git.css';
import { GitSection } from './GitSection';
import { LhSection } from './LhSection';
import { PlanSection } from './PlanSection';
import { RunningSummary } from './RunningSummary';
import './status.css';

/** The toast strip the sections' actions report through. */
function useToast() {
  const [toast, setToast] = createSignal<string | undefined>(undefined);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const show = (text: string) => {
    setToast(text);
    clearTimeout(timer);
    timer = setTimeout(() => setToast(undefined), 3000);
  };
  onCleanup(() => clearTimeout(timer));
  return { toast, show };
}

function Toast(props: { text?: string }) {
  return (
    <Show when={props.text}>
      <div class="git-toast" role="status" data-testid="git-toast">{props.text}</div>
    </Show>
  );
}

/** A pending "scroll to this section" request (the pane may already be open). */
export type StatusSectionId = 'lh' | 'plan' | 'running';
const [sectionFocus, setSectionFocus] = createSignal<StatusSectionId | undefined>(undefined);
export const statusSectionFocus = sectionFocus;

/**
 * Opens/focuses the status pane — the side-pane `git` tab on the desktop
 * (Z-100), this PC's full-screen layer on the phone — and asks it to scroll
 * to `section` when given.
 */
export function openStatusSection(base: { hostId: string; sessionId?: string; path: string }, section?: StatusSectionId): void {
  setSectionFocus(section);
  if (desktopShell()) {
    const ws = currentWorkspace();
    if (ws) {
      panes().open(ws.scope, 'side', ws.view, { kind: 'git', params: gitParams({ hostId: base.hostId, path: base.path }) });
      layout().toggleSide(true);
      return;
    }
  }
  openGit({ hostId: base.hostId, path: base.path });
}

const SOURCES: GitSource[] = ['unstaged', 'staged', 'branch', 'turn'];

/**
 * The pane tab body: `hostId` + the project (the session cwd) tell Git where
 * to run; `sessionId` feeds the "Last turn" source and the A16 sections
 * (they are session-scoped — no session, no sections). `params.path` carries
 * the folder the pane was opened at before the first status resolves the
 * repo root.
 */
export function StatusPane(props: PaneTabProps) {
  const { toast, show } = useToast();
  // `params.path` is the folder the pane was opened at (Files chip / chat ⋯
  // Git); else the project key — the session cwd, or `~` (the host expands
  // it to home) for a draft without one.
  const path = () => {
    const p = props.tab.params?.path;
    return typeof p === 'string' && p ? p : props.projectKey || '~';
  };
  const source = () => {
    const s = props.tab.params?.source;
    return SOURCES.find((x) => x === s);
  };
  const tab = () => (props.tab.params?.tab === 'history' ? 'history' : undefined);
  return (
    <div class="status-pane" data-testid="status-pane">
      <div class="status-pane-scroll">
        <GitSection
          hostId={props.hostId}
          path={path()}
          sessionId={props.sessionId}
          cwd={path()}
          initialSource={source()}
          initialTab={tab()}
          onToast={show}
        />
        <StatusSections hostId={props.hostId} sessionId={props.sessionId} onToast={show} />
      </div>
      <Toast text={toast()} />
    </div>
  );
}

setPaneComponent('git', StatusPane);

/** The A16 sections (LH, Plan & todos, Running summary) — session-scoped. */
export function StatusSections(props: { hostId: string; sessionId?: string; onToast?: (text: string) => void }) {
  let lh: HTMLDivElement | undefined;
  let plan: HTMLDivElement | undefined;
  let running: HTMLDivElement | undefined;
  createEffect(() => {
    const want = sectionFocus();
    if (!want) return;
    setSectionFocus(undefined);
    queueMicrotask(() => {
      const el = want === 'lh' ? lh : want === 'plan' ? plan : running;
      el?.scrollIntoView({ behavior: 'smooth', block: 'start' });
    });
  });
  return (
    <Show when={props.sessionId}>
      {(id) => (
        <div class="status-sections" data-testid="status-sections">
          <div ref={lh}>
            <LhSection hostId={props.hostId} sessionId={id()} onToast={props.onToast} />
          </div>
          <div ref={plan}>
            <PlanSection hostId={props.hostId} sessionId={id()} />
          </div>
          <div ref={running}>
            <RunningSummary hostId={props.hostId} sessionId={id()} />
          </div>
        </div>
      )}
    </Show>
  );
}

/**
 * The phone's full-screen status layer (§3.9): the same sections in the
 * unipi phone chrome (56 px header, Android Back closes). Mounted once by
 * the shell, driven by `lib/git.ts` `gitTarget()`.
 */
export function StatusPaneLayer() {
  const close = () => closeGit();
  return (
    <Show when={gitTarget()} keyed>
      {(target) => (
        <StatusLayerBody hostId={target.hostId} path={target.path} sessionId={target.sessionId} source={target.source} tab={target.tab} onClose={close} />
      )}
    </Show>
  );
}

function StatusLayerBody(props: { hostId: string; path: string; sessionId?: string; source?: GitSource; tab?: 'changes' | 'history'; onClose: () => void }) {
  const { toast, show } = useToast();
  onMount(() => {
    pushBack(props.onClose);
  });
  return (
    <div class="git-view status-pane-layer" role="dialog" aria-label="Git and status" data-testid="git-view">
      <div class="status-pane-scroll">
        <GitSection
          hostId={props.hostId}
          path={props.path}
          sessionId={props.sessionId}
          cwd={props.path}
          onBack={props.onClose}
          onToast={show}
          initialSource={props.source}
          initialTab={props.tab}
        />
        <StatusSections hostId={props.hostId} sessionId={props.sessionId} onToast={show} />
      </div>
      <Toast text={toast()} />
    </div>
  );
}
