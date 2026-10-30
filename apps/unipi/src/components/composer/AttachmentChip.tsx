/**
 * Attachment chips (WP-A13, Z-60 states): one chip per pending attachment in
 * the row above the composer — a thumbnail for an image (tap = annotate,
 * D-5), an icon + name + size for a file, and the upload state on top of it:
 *
 *   queued / uploading  a thin progress bar + the percentage (ZCode caps at
 *                       99 % until the upload commits, so 100 % means done);
 *   failed              red border, the reason in the title, a Retry button;
 *   ready               plain chip, ✕ removes it.
 *
 * Pure presentation — the store is `lib/composer/attachments.ts`.
 */
import { AlertCircle, FileText, ImageOff, Pencil, RotateCw } from 'lucide-solid';
import { Show } from 'solid-js';
import type { PendingAttachment } from '../../lib/composer/attachments';

/** 1.2 MB, 830 B — the chip's size hint. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(bytes < 10 * 1024 * 1024 ? 1 : 0)} MB`;
}

const percent = (progress: number): number => Math.max(0, Math.min(100, Math.round(progress * 100)));

export function AttachmentChip(props: {
  attachment: PendingAttachment;
  onRemove: () => void;
  /** Manual retry after an upload failed (Z-60). */
  onRetry: () => void;
  /** Images: annotate in the image editor (D-5). */
  onEdit?: () => void;
}) {
  const a = () => props.attachment;
  const busy = () => a().status === 'queued' || a().status === 'uploading';
  const failed = () => a().status === 'failed';
  const stateLabel = () =>
    failed() ? 'failed' : a().status === 'ready' ? 'ready' : a().status === 'uploading' ? `${percent(a().progress)} %` : 'waiting';

  return (
    <span
      class="att-chip"
      data-kind={a().kind}
      data-status={a().status}
      data-testid="attachment-chip"
      role="listitem"
      aria-label={`${a().name} — ${stateLabel()}`}
      title={failed() ? `${a().name}: ${a().error ?? 'upload failed'}` : `${a().name} · ${formatBytes(a().size)}`}
    >
      <Show
        when={a().kind === 'image' && a().preview}
        fallback={
          <span class="att-ic">
            {a().kind === 'image' ? <ImageOff size={14} /> : <FileText size={14} />}
          </span>
        }
      >
        {(src) => (
          <button type="button" class="att-thumb" aria-label={`Edit ${a().name}`} disabled={busy() || !props.onEdit} onClick={() => props.onEdit?.()}>
            <img src={src()} alt="" />
            <Show when={props.onEdit && !busy() && !failed()}>
              <span class="att-edit" aria-hidden="true">
                <Pencil size={10} />
              </span>
            </Show>
          </button>
        )}
      </Show>
      <span class="att-body">
        <span class="att-name">{a().name}</span>
        <span class="att-meta" data-testid="attachment-state">
          {failed() ? (a().error ?? 'Upload failed') : busy() ? `${a().status === 'queued' ? 'Waiting' : 'Uploading'} ${percent(a().progress)} %` : formatBytes(a().size)}
        </span>
        <Show when={busy()}>
          <span class="att-bar" aria-hidden="true">
            <span class="att-bar-fill" style={{ width: `${percent(a().progress)}%` }} />
          </span>
        </Show>
      </span>
      <Show when={failed()}>
        <button type="button" class="att-act att-retry" aria-label={`Retry ${a().name}`} onClick={() => props.onRetry()} data-testid="attachment-retry">
          <RotateCw size={13} />
        </button>
      </Show>
      <Show when={failed()}>
        <span class="att-fail" aria-hidden="true">
          <AlertCircle size={12} />
        </span>
      </Show>
      <button type="button" class="att-act att-remove" aria-label={`Remove ${a().name}`} onClick={() => props.onRemove()} data-testid="attachment-remove">
        ✕
      </button>
    </span>
  );
}
