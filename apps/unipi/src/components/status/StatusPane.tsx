/**
 * The status pane (WP-A15): the `git` side-pane tab (reserved `host/git`,
 * §3.8) — today the Git section (Z-96..Z-100); WP-A16 adds the long-horizon,
 * plan & todos and Running sections under the same pane. On the phone the
 * pane is the full-screen layer of §3.9 (`StatusPaneLayer`, back button).
 *
 * One Git UI (Z-100): everything opens this pane — the `+` launcher, the
 * Files branch chip, the chat ⋯ Git entry, Cmd/Ctrl+Alt+B (the side-pane
 * toggle). The collapsed capsule arrives with A16.
 */
import { createSignal, onCleanup, onMount, Show } from 'solid-js';
import { ArrowLeft } from 'lucide-solid';
import type { PaneTabProps } from '../../lib/panes/kinds';
import { setPaneComponent } from '../../lib/panes/kinds';
import { pushBack } from '../../lib/back';
import { closeGit, gitTarget, type GitSource } from '../../lib/git';
import '../git/git.css';
import { GitSection } from './GitSection';

/** The toast strip the Git section's actions report through. */
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

/**
 * The pane tab body: `hostId` + the project (the session cwd) tell Git where
 * to run; `sessionId` feeds the "Last turn" source. `params.path` carries the
 * folder the pane was opened at (Files chip / chat ⋯ Git) before the first
 * status resolves the repo root.
 */
const SOURCES: GitSource[] = ['unstaged', 'staged', 'branch', 'turn'];

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
      <GitSection
        hostId={props.hostId}
        path={path()}
        sessionId={props.sessionId}
        cwd={path()}
        initialSource={source()}
        initialTab={tab()}
        onToast={show}
      />
      <Toast text={toast()} />
    </div>
  );
}

setPaneComponent('git', StatusPane);

/**
 * The phone's full-screen Git layer (§3.9): the same sections in the unipi
 * phone chrome (56 px header, Android Back closes). Mounted once by the
 * shell, driven by `lib/git.ts` `gitTarget()`.
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
    <div class="git-view status-pane-layer" role="dialog" aria-label="Git" data-testid="git-view">
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
      <Toast text={toast()} />
    </div>
  );
}
