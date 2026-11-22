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
import { createSignal, Match, onCleanup, onMount, Show, Switch } from 'solid-js';
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
import { TodoSection } from './TodoParts';
import { openGraphView } from './GraphView';
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

/** The status sections a pane tab (or the phone layer) can show — one at a time. */
export type StatusSectionId = 'lh' | 'plan' | 'running';
const [layerSection, setLayerSection] = createSignal<StatusSectionId | undefined>(undefined);

/**
 * Opens the Git tab (no `section`) or ONE status section — the long-horizon
 * graph, Plan & todos, Running — as its own side-pane tab (`status` kind) on
 * the desktop, or this PC's full-screen layer on the phone. Sections are never
 * stacked under Git: each tab renders only its own content.
 */
export function openStatusSection(base: { hostId: string; sessionId?: string; path: string }, section?: StatusSectionId): void {
  if (desktopShell()) {
    const ws = currentWorkspace();
    if (ws) {
      const params = section
        ? { section, hostId: base.hostId, path: base.path }
        : gitParams({ hostId: base.hostId, path: base.path });
      panes().open(ws.scope, 'side', ws.view, { kind: section ? 'status' : 'git', params });
      layout().toggleSide(true);
      return;
    }
  }
  if (section === 'lh') {
    if (base.sessionId) return openGraphView({ hostId: base.hostId, sessionId: base.sessionId, path: base.path });
  }
  setLayerSection(section);
  openGit({ hostId: base.hostId, path: base.path, ...(base.sessionId ? { sessionId: base.sessionId } : {}) });
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
      </div>
      <Toast text={toast()} />
    </div>
  );
}

setPaneComponent('git', StatusPane);

/** The `status` pane tab: exactly one session-scoped section (`params.section`). */
export function StatusSectionPane(props: PaneTabProps) {
  const { toast, show } = useToast();
  const section = (): StatusSectionId => {
    const v = props.tab.params?.section;
    return v === 'plan' || v === 'running' ? v : 'lh';
  };
  return (
    <div class="status-pane" data-testid="status-pane" data-section={section()}>
      <div class="status-pane-scroll">
        <StatusSections section={section()} hostId={props.hostId} sessionId={props.sessionId} onToast={show} />
      </div>
      <Toast text={toast()} />
    </div>
  );
}

setPaneComponent('status', StatusSectionPane);

/** One A16 section (LH graph, Plan & todos, or Running) — session-scoped. */
export function StatusSections(props: { section?: StatusSectionId; hostId: string; sessionId?: string; onToast?: (text: string) => void }) {
  return (
    <Show
      when={props.sessionId}
      fallback={<p class="status-empty" data-testid="status-no-session">Open a chat to see this.</p>}
    >
      {(id) => (
        <div class="status-sections" data-testid="status-sections">
          <Switch>
            <Match when={(props.section ?? 'lh') === 'lh'}>
              <TodoSection hostId={props.hostId} sessionId={id()} />
              <LhSection hostId={props.hostId} sessionId={id()} onToast={props.onToast} />
            </Match>
            <Match when={props.section === 'plan'}>
              <PlanSection hostId={props.hostId} sessionId={id()} />
            </Match>
            <Match when={props.section === 'running'}>
              <RunningSummary hostId={props.hostId} sessionId={id()} />
            </Match>
          </Switch>
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
  // Back closes the layer; the pop MUST ride the cleanup, or a stale handler
  // stays on the stack and swallows the next Back after the layer is gone.
  onMount(() => onCleanup(pushBack(props.onClose)));
  const section = layerSection();
  onCleanup(() => setLayerSection(undefined));
  return (
    <div class="git-view status-pane-layer" role="dialog" aria-label="Git and status" data-testid="git-view">
      <div class="status-pane-scroll">
        <Show
          when={section}
          fallback={
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
          }
        >
          {(sec) => (
            <>
              <button type="button" class="status-layer-back" onClick={props.onClose} aria-label="Back"><ArrowLeft size={16} /> Back</button>
              <StatusSections section={sec()} hostId={props.hostId} sessionId={props.sessionId} onToast={show} />
            </>
          )}
        </Show>
      </div>
      <Toast text={toast()} />
    </div>
  );
}
