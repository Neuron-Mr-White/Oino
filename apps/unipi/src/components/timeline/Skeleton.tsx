/**
 * Loading skeleton (WP-A6, Z-88 — unipi's kept): `ChatSkeleton` shows while
 * the stream is `connecting` and no entries are in yet. (Image chips
 * already shimmer via `.chat-image-chip.is-loading` in media.css.)
 */
export function ChatSkeleton() {
  return (
    <div class="tl-skeleton chat-skeleton" data-testid="tl-skeleton" aria-label="Loading the conversation">
      <span />
      <span />
      <span />
    </div>
  );
}
