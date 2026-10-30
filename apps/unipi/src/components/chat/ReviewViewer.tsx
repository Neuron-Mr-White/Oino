/**
 * Full-screen review viewer (UNI-192): a read-only diff for one
 * "Changes this turn" file, opened at a chosen edit, with +/- line numbers
 * (the same diffview as the inline card) and a pager across the file's
 * other edits. Lives inside the chat layer (a Portal, like
 * MermaidViewer/Previewer); Back (lib/back.ts pushBack) closes it.
 * "Open in editor" in the header/footer jumps to the same spot in the real
 * editor — it reads the open chat's hostId/cwd itself (`openChatState`,
 * like `openChat`'s siblings) so the viewer mounts once in main.tsx with no
 * wiring through ChatView.tsx.
 */
import { createMemo, createSignal, onCleanup, onMount, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { useNavigate } from '@solidjs/router';
import { ArrowLeft, ChevronLeft, ChevronRight, ExternalLink } from 'lucide-solid';
import { pushBack } from '../../lib/back';
import { editDiff, renderDiffHtml } from '../../lib/markdown';
import { resolvePathToken } from '../../lib/media/pathTokens';
import { setEditorTarget } from '../../lib/editorTarget';
import { openChatState } from '../../lib/session/open';
import { turnFileEditLine, type TurnFile } from '../../lib/chat/model';

export type ReviewRequest = { file: TurnFile; editIndex: number };

const [request, setRequest] = createSignal<ReviewRequest | undefined>(undefined);

/** Opens the review viewer for `file`, starting at `editIndex` (0 if omitted). */
export function openReviewViewer(file: TurnFile, editIndex = 0): void {
  setRequest({ file, editIndex });
}

export function closeReviewViewer(): void {
  setRequest(undefined);
}

export function ReviewViewer() {
  return (
    <Show when={request()}>
      {(req) => <ReviewLayer request={req()} onClose={closeReviewViewer} />}
    </Show>
  );
}

function ReviewLayer(props: { request: ReviewRequest; onClose: () => void }) {
  const navigate = useNavigate();
  const [index, setIndex] = createSignal(props.request.editIndex);
  onMount(() => onCleanup(pushBack(props.onClose)));

  const edits = () => props.request.file.edits;
  const single = () => edits().length <= 1;
  const current = () => edits()[index()];
  const html = createMemo(() => {
    const e = current();
    if (!e) return '';
    return renderDiffHtml({ files: [{ oldPath: props.request.file.path, newPath: props.request.file.path, hunks: [editDiff(e.oldText, e.newText)] }] });
  });

  const openInEditor = () => {
    const open = openChatState();
    const hostId = open?.chat.hostId;
    if (!hostId) return;
    const file = props.request.file;
    const edit = file.edits[index()] ?? file.edits[0]!;
    const line = turnFileEditLine(edit);
    const cwd = open?.chat.state?.session?.cwd;
    setEditorTarget({ hostId, path: resolvePathToken(file.path, cwd), line });
    // A Portal over the chat layer (UNI-192 bug, same cause as the
    // Show-all drawer not opening the editor): close it *before* the
    // editor route mounts, so it doesn't sit on top of it.
    props.onClose();
    navigate('/files/edit');
  };

  return (
    <Portal>
      <div class="review-viewer" role="dialog" aria-label={`Review ${props.request.file.path}`} data-testid="review-viewer">
        <header class="review-viewer-head detail-head">
          <button type="button" class="ghost-icon-button" aria-label="Close review" onClick={props.onClose}>
            <ArrowLeft size={22} />
          </button>
          <div class="review-viewer-title">
            <span class="review-viewer-path truncate">{props.request.file.path}</span>
            <Show when={!single()}>
              <span class="review-viewer-edit-count">
                edit {index() + 1} / {edits().length}
              </span>
            </Show>
          </div>
          <button type="button" class="ghost-icon-button" aria-label="Open in editor" onClick={openInEditor}>
            <ExternalLink size={20} />
          </button>
        </header>
        <div class="review-viewer-body">
          <div class="tool-diff diffview-wrap" innerHTML={html()} />
        </div>
        <Show when={!single()}>
          <footer class="review-viewer-foot">
            <button
              type="button"
              class="ghost-icon-button"
              aria-label="Previous edit"
              disabled={index() === 0}
              onClick={() => setIndex((i) => Math.max(0, i - 1))}
            >
              <ChevronLeft size={20} />
            </button>
            <span class="review-viewer-edit-count">
              edit {index() + 1} of {edits().length}
            </span>
            <button
              type="button"
              class="ghost-icon-button"
              aria-label="Next edit"
              disabled={index() === edits().length - 1}
              onClick={() => setIndex((i) => Math.min(edits().length - 1, i + 1))}
            >
              <ChevronRight size={20} />
            </button>
          </footer>
        </Show>
      </div>
    </Portal>
  );
}
