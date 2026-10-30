/**
 * Chat markdown (UNI-113, UNI-156): assistant prose → sanitised HTML.
 * Re-exports the shared renderer (lib/markdown) — see there for the
 * code-block header, diff/mermaid fences and the streaming block cache.
 * Links open outside the app (the view intercepts clicks).
 */
export { renderMarkdown as renderChatMarkdown, splitBlocks, renderBlocks, hydrateCode } from '../markdown';
