/**
 * Loading skeleton (WP-A6, Z-88): `ChatSkeleton` shows only while `connecting`
 * when no cached turn snapshot exists yet for this session. Subtle inline
 * conversation silhouette (query bubble + folded work line + two prose lines).
 */
export function ChatSkeleton() {
  return (
    <div class="tl-skeleton chat-skeleton" data-testid="tl-skeleton" aria-label="Loading the conversation">
      <span class="tl-skel-bubble" />
      <span class="tl-skel-fold" />
      <span class="tl-skel-line tl-skel-w90" />
      <span class="tl-skel-line tl-skel-w68" />
    </div>
  );
}
