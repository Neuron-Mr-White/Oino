/**
 * One row of the Files tree (WP-A20, Z-101): compact 26 px row — indent,
 * chevron, typed file icon (the unipi document families), name, git badge
 * (M/A/D/?), gitignored rows dimmed, dotfiles shown. The row itself stays
 * dumb: selection, menus and keys live in `FileTree`.
 */
import { Show, Switch, Match } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import {
  ChevronRight,
  FileArchive,
  FileCode,
  FileIcon,
  FileImage,
  FileSpreadsheet,
  FileTerminal,
  FileText,
  FileVideoCamera,
  Folder,
  Loader2,
  MoreHorizontal,
  Music,
  Presentation,
  FileSymlink,
} from 'lucide-solid';
import type { TreeRow } from '../../lib/files/tree';
import type { IconComponent } from '../../lib/panes/kinds';
import { diagFor } from '../../lib/lsp/diagnostics';

const IMAGE_EXTS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'bmp', 'avif', 'ico']);
const VIDEO_EXTS = new Set(['mp4', 'mkv', 'webm', 'mov', 'avi', 'm4v']);
const AUDIO_EXTS = new Set(['mp3', 'ogg', 'wav', 'flac', 'm4a', 'opus', 'aac']);
const SHEET_EXTS = new Set(['xls', 'xlsx', 'ods', 'csv', 'tsv']);
const SLIDE_EXTS = new Set(['ppt', 'pptx', 'odp', 'key']);
const DOC_EXTS = new Set(['doc', 'docx', 'odt', 'rtf', 'pages', 'pdf']);
const ARCHIVE_EXTS = new Set(['zip', 'tar', 'gz', 'tgz', 'bz2', 'xz', '7z', 'rar', 'apk', 'jar', 'deb', 'rpm', 'iso']);
const CODE_EXTS = new Set([
  'js', 'jsx', 'mjs', 'cjs', 'ts', 'tsx', 'py', 'rs', 'sh', 'bash', 'zsh', 'yml', 'yaml', 'html', 'htm',
  'c', 'cc', 'cpp', 'h', 'hpp', 'json', 'css', 'scss', 'sql', 'go', 'java', 'cs', 'rb', 'toml', 'lua', 'vim',
]);

/** Typed row icons (UNI-166): every document family gets its own glyph. */
export function fileIcon(name: string): IconComponent {
  const ext = name.slice(name.lastIndexOf('.') + 1).toLowerCase();
  if (IMAGE_EXTS.has(ext)) return FileImage;
  if (VIDEO_EXTS.has(ext)) return FileVideoCamera;
  if (AUDIO_EXTS.has(ext)) return Music;
  if (SHEET_EXTS.has(ext)) return FileSpreadsheet;
  if (SLIDE_EXTS.has(ext)) return Presentation;
  if (ARCHIVE_EXTS.has(ext)) return FileArchive;
  if (CODE_EXTS.has(ext)) return FileCode;
  if (ext === 'log' || ext === 'logs') return FileTerminal;
  if (DOC_EXTS.has(ext) || ext === 'md' || ext === 'markdown' || ext === 'txt') return FileText;
  return FileIcon;
}

export const GIT_LABEL: Record<string, string> = { M: 'Modified', A: 'Added', D: 'Deleted', '?': 'Untracked' };

export function TreeRowView(props: {
  row: TreeRow;
  selected: boolean;
  onActivate: (row: TreeRow) => void;
  onMenu: (row: TreeRow, anchor: DOMRect) => void;
  /** Inline rename (F2 / menu "Rename…"): the name becomes an input. */
  renaming?: boolean;
  /** Commit (`name`) or cancel (`undefined`) the inline rename. */
  onRenameDone?: (name: string | undefined) => void;
}) {
  const row = () => props.row;
  const isDir = () => row().kind === 'dir' || row().kind === 'root';
  const diag = () => diagFor(row().path);
  return (
    <div
      class="ftree-row"
      role="treeitem"
      aria-level={row().depth + 1}
      aria-selected={props.selected}
      aria-expanded={isDir() ? !!row().expanded : undefined}
      data-kind={row().kind}
      data-selected={props.selected ? '' : undefined}
      data-ignored={row().ignored ? '' : undefined}
      data-test-row={row().path}
      tabindex={-1}
      style={{ 'padding-left': `${6 + row().depth * 14}px` }}
      title={row().error ? `${row().path} — ${row().error}` : row().path}
      onClick={() => {
        if (!props.renaming) props.onActivate(row());
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        props.onMenu(row(), (e.currentTarget as HTMLElement).getBoundingClientRect());
      }}
      onKeyDown={(e) => {
        if (props.renaming) return;
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          props.onActivate(row());
        }
      }}
    >
      <Show when={isDir()} fallback={<span class="ftree-twist" aria-hidden="true" />}>
        <span class="ftree-twist" data-open={row().expanded ? '' : undefined} aria-hidden="true">
          <Show when={!row().loading} fallback={<Loader2 size={12} class="spin" />}>
            <ChevronRight size={12} strokeWidth={2.2} />
          </Show>
        </span>
      </Show>
      <span class="ftree-icon" aria-hidden="true">
        <Dynamic component={isDir() ? Folder : row().kind === 'symlink' ? FileSymlink : fileIcon(row().name)} size={15} strokeWidth={1.7} />
      </span>
      <Show when={props.renaming}>
        <RenameInput name={row().name} isDir={isDir()} onDone={(name) => props.onRenameDone?.(name)} />
      </Show>
      <span class="ftree-name" data-error={row().error ? '' : undefined} hidden={props.renaming}>
        {row().name}
        <Show when={row().empty}>
          <span class="ftree-empty"> (empty)</span>
        </Show>
        <Show when={row().more}>
          <span class="ftree-more"> …</span>
        </Show>
      </span>
      <Switch>
        <Match when={row().error}>
          <span class="ftree-err" role="note">unavailable</span>
        </Match>
      </Switch>
      <Show when={!isDir() && diag() !== undefined}>
        <span
          class="ftree-diag"
          classList={{ err: !!diag()!.errors }}
          aria-label={diag()!.errors ? `${diag()!.errors} errors` : `${diag()!.warnings} warnings`}
        >
          {diag()!.errors || diag()!.warnings}
        </span>
      </Show>
      <Show when={row().git}>
        <span class="ftree-git" data-git={row().git} aria-label={GIT_LABEL[row().git ?? ''] ?? row().git}>
          {row().git}
        </span>
      </Show>
      <button
        type="button"
        class="ftree-menu"
        aria-label={`Actions for ${row().name}`}
        tabindex={-1}
        onClick={(e) => {
          e.stopPropagation();
          props.onMenu(row(), (e.currentTarget as HTMLElement).getBoundingClientRect());
        }}
      >
        <MoreHorizontal size={14} />
      </button>
    </div>
  );
}

/** The tree's inline rename field: the stem is pre-selected (extension
 *  kept), Enter / blur commit, Escape cancels. */
function RenameInput(props: { name: string; isDir: boolean; onDone: (name: string | undefined) => void }) {
  let done = false;
  const finish = (name: string | undefined) => {
    if (done) return;
    done = true;
    props.onDone(name);
  };
  return (
    <input
      class="ftree-rename"
      data-testid="files-rename-input"
      aria-label={`Rename ${props.name}`}
      value={props.name}
      spellcheck={false}
      autocomplete="off"
      ref={(el) =>
        queueMicrotask(() => {
          el.focus({ preventScroll: true });
          const dot = props.isDir ? -1 : props.name.lastIndexOf('.');
          el.setSelectionRange(0, dot > 0 ? dot : props.name.length);
        })
      }
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === 'Enter') {
          e.preventDefault();
          finish(e.currentTarget.value);
        } else if (e.key === 'Escape') {
          e.preventDefault();
          finish(undefined);
        }
      }}
      onBlur={(e) => finish(e.currentTarget.value)}
    />
  );
}
