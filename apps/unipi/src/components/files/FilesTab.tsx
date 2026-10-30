/**
 * The `files` pane tab (WP-A20, reserved `host/files`): the editor area
 * left, the tree right, resizable (ZCode geometry, audit Z-101). On the
 * phone shell the tab fills the screen as the `/files` layer (§3.9) and
 * file opens go through the same editor-target slot (the `/files/edit`
 * screen). Media opens the `preview` tab (desktop) or the previewer layer
 * (phone). Registered as the `files` kind's view below — one line, per
 * `lib/panes/kinds.ts`.
 */
import { createEffect, createResource, createSignal, onMount, Show } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { ArrowLeft } from 'lucide-solid';
import { setEditorTarget, switchEditorTarget, getEditorTarget, type EditorTarget } from '../../lib/editorTarget';
import { desktopShell } from '../../lib/layout';
import { consumeFilesTarget } from '../../lib/filesTarget';
import { fsRoots } from '../../lib/fs';
import { openPreviewer } from '../../lib/media/previewerState';
import type { PaneScope, PaneView } from '../../lib/panes';
import { openSideTab } from '../panes/SidePane';
import { currentWorkspace } from '../shell/Workspace';
import { ResizeHandle } from '../shell/ResizeHandle';
import { NoHostState } from '../EmptyState';
import { setAddHostOpen } from '../Pairing';
import { useHostsOptional } from '../../hosts/context';
import { LOCAL_HOST } from '../../lib/host/client';
import { EditorArea } from './EditorArea';
import { FileTree } from './FileTree';
import { setPaneComponent, type PaneTabProps } from '../../lib/panes/kinds';
import { pushBack } from '../../lib/back';
import './files.css';

const TREE_W_KEY = 'unipi.files.treeW';
const TREE_DEFAULT = 232;
const TREE_MIN = 160;
const TREE_MAX = 520;

function readTreeW(): number {
  const raw = Number(localStorage.getItem(TREE_W_KEY));
  return Number.isFinite(raw) && raw >= TREE_MIN && raw <= TREE_MAX ? raw : TREE_DEFAULT;
}

export function FilesTab(props: PaneTabProps) {
  const navigate = useNavigate();
  const hosts = useHostsOptional();
  const hostId = () => String(props.tab.params?.hostId ?? props.hostId);
  const root = () => {
    const key = String(props.tab.params?.root ?? props.projectKey);
    return key && key !== '~' ? key : undefined;
  };
  const [roots] = createResource(hostId, (id) => fsRoots(id));
  const rootList = () => {
    const r = roots();
    return r?.ok ? r.value.roots : undefined;
  };

  const [treeW, setTreeW] = createSignal(readTreeW());
  const scope = (): PaneScope => currentWorkspace()?.scope ?? { hostId: hostId(), projectKey: props.projectKey };
  const view = (): PaneView => currentWorkspace()?.view ?? {};

  function openFile(path: string, opts?: { line?: number; asText?: boolean }) {
    const next: EditorTarget = { hostId: hostId(), path, line: opts?.line, asText: opts?.asText };
    if (desktopShell()) {
      // A dirty editor asks first through its guard (UNI-117).
      switchEditorTarget(next);
      return;
    }
    setEditorTarget(next);
    navigate('/files/edit');
  }

  function preview(path: string, siblings: string[]) {
    if (desktopShell()) {
      openSideTab(scope(), view(), { kind: 'preview', params: { hostId: hostId(), path } });
      return;
    }
    const index = Math.max(0, siblings.indexOf(path));
    openPreviewer({ kind: 'list', hostId: hostId(), paths: siblings.length ? siblings : [path], index });
  }

  createEffect(() => {
    // "Files here" (chat header) / capture target for this host: open it once.
    const target = consumeFilesTarget();
    if (target && target.hostId === hostId()) setEditorTarget(target);
    // A seeded tab (deep link, scenario) starts with this file open.
    const seed = props.tab.params?.path;
    if (seed && !getEditorTarget()) setEditorTarget({ hostId: hostId(), path: String(seed) });
  });

  const phoneLayer = () => !desktopShell();
  const noHost = () => hosts !== undefined && !hosts.hosts().some((h) => h.id === hostId());

  return (
    <Show when={!noHost()} fallback={<NoHostState what="Files" onAdd={() => setAddHostOpen(true)} />}>
      <div class="files-tab" data-testid="files-tab" data-shell={phoneLayer() ? 'phone' : 'desktop'}>
        <Show
          when={!phoneLayer()}
          fallback={
            <div class="files-tab-tree alone">
              <FileTree hostId={hostId()} root={root()} roots={rootList} openFile={openFile} onPreview={preview} />
            </div>
          }
        >
          <div class="files-tab-editor">
            <EditorArea onBinary={(path) => preview(path, [path])} />
          </div>
          <ResizeHandle
            axis="x"
            dir={-1}
            label="Resize file tree"
            class="ftree-handle"
            size={treeW()}
            min={TREE_MIN}
            max={TREE_MAX}
            onSize={(w) => {
              const clamped = Math.min(TREE_MAX, Math.max(TREE_MIN, w));
              setTreeW(clamped);
              try {
                localStorage.setItem(TREE_W_KEY, String(Math.round(clamped)));
              } catch {
                // best effort
              }
            }}
            onReset={() => setTreeW(TREE_DEFAULT)}
            testId="files-tree-handle"
          />
          <div class="files-tab-tree" style={{ width: `${treeW()}px` }}>
            <FileTree hostId={hostId()} root={root()} roots={rootList} openFile={openFile} onPreview={preview} />
          </div>
        </Show>
      </div>
    </Show>
  );
}

setPaneComponent('files', FilesTab);

/**
 * The phone `/files` route (§3.9: a side-pane tab is a full-screen layer
 * there): the same tree, full-bleed, on the phone's selected host, in the
 * unipi detail chrome (56 px header with Back, Android Back closes it) so
 * the layer can be left the same way the editor returns to it. Reached from
 * the chat header "Files here" until A29 adds the ⋯ "Panes" group; the
 * bottom-nav Files item is gone (Z-101: side-pane tab only).
 */
export function FilesLayer() {
  const hosts = useHostsOptional();
  const navigate = useNavigate();
  const hostId = () => hosts?.selectedId() ?? LOCAL_HOST;
  const close = () => (window.history.length > 1 ? navigate(-1) : navigate('/'));
  onMount(() => pushBack(close));
  return (
    <div class="files-layer" role="dialog" aria-label="Files" data-testid="files-layer">
      <header class="files-layer-head">
        <button type="button" class="ftree-tool" aria-label="Back" data-testid="files-layer-back" onClick={close}>
          <ArrowLeft size={18} />
        </button>
        <h1 class="files-layer-title">Files</h1>
      </header>
      <FilesTab
        tab={{ id: 'files-layer', kind: 'files', params: { hostId: hostId() } }}
        area="side"
        hostId={hostId()}
        projectKey="~"
        close={close}
        setTitle={() => undefined}
      />
    </div>
  );
}
