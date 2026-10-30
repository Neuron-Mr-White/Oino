/**
 * Shared markdown renderer (UNI-156): one marked+DOMPurify pipeline, one
 * `.md` stylesheet (style.css tokens), for chat prose/thinking/cards and
 * board bodies/reports/summaries alike. `lib/chat/markdown.ts` and
 * `lib/board/markdown.ts` re-export their call sites' slice of this.
 */
export { renderMarkdown, hydrateCode, type RenderOptions } from './core';
export { splitBlocks, renderBlocks, resetBlockCache } from './blocks';
export { plainText } from './plain';
export { isFenceClosed } from './fence';
export {
  attKind,
  attDisplayName,
  attachmentHtml,
  attachmentLinkHtml,
  hydrateAttachments,
  ATT_REF,
  type AttKind,
} from './attachments';
export { parseUnifiedDiff, renderDiffHtml, diffCounts, wordMarks, allAdditions, editDiff, isDiffLang, type ParsedDiff, type DiffFile, type DiffHunk, type DiffLine } from './diff';
export { canonicalLang, isSupportedLang, highlightHtml, resetHighlightCache, langForPath, languageForPath } from './highlight';
// `renderMermaid` itself only ever `import()`s the (huge) `mermaid` package
// lazily (lib/markdown/core.ts's hydrateCode calls it on demand, and tests
// import it directly from ./mermaid) — `isMermaidLang` is synchronous and
// needed at render time, so only it (plus the type) is re-exported here.
export { isMermaidLang, type MermaidResult } from './mermaid';
