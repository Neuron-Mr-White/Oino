/**
 * Tool-row data helpers (WP-A8): the `_oino/render` payload of a tool
 * (`{toolCall, args, result?, partialResult?, isError?}` — the shape
 * utility's `ToolView::from_data` parses), the images of a tool result
 * (Z-44 chips), and the lazy full output (`get_entries`, §3.4 step 7).
 */
import type { ToolRun } from '../../../lib/session/turns';
import type { SessionEntry } from '../../../lib/session/types';
import type { TimelineCtx } from '../context';
import { imagesOf, type ImageRef } from '../../../lib/chat/model';

/** The `_oino/render{target:"tool"}` data payload of a run. */
export function toolRenderData(tool: ToolRun): Record<string, unknown> {
  const out: Record<string, unknown> = { toolCall: { id: tool.id, name: tool.name }, args: tool.args };
  if (tool.state === 'running' || tool.state === 'preparing') {
    if (tool.live !== undefined) out.partialResult = { content: [{ type: 'text', text: tool.live }] };
  } else {
    out.result = {
      content: [{ type: 'text', text: tool.output ?? '' }],
      ...(tool.details !== undefined ? { details: tool.details } : {}),
      isError: tool.state === 'error',
    };
  }
  if (tool.state === 'error') out.isError = true;
  return out;
}

/** The toolResult entry of a run (`resultId`), from the session's index. */
export function resultEntryOf(ctx: TimelineCtx, tool: ToolRun): SessionEntry | undefined {
  const id = tool.resultId;
  if (!id) return undefined;
  const at = ctx.session.state.byId[id];
  return at === undefined ? undefined : ctx.session.state.entries[at];
}

/** Image parts of a tool result (Z-44: chips + previewer kept). */
export function toolImages(ctx: TimelineCtx, tool: ToolRun): ImageRef[] {
  const entry = resultEntryOf(ctx, tool);
  const content = entry?.message?.content;
  return Array.isArray(content) ? imagesOf(content) : [];
}

const FULL_MAX = 200;
const fullOutputs = new Map<string, string>();

/** Remembers one entry's full output text (LRU, test seam `clearFullOutputs`). */
export function rememberFullOutput(entryId: string, text: string): void {
  fullOutputs.delete(entryId);
  fullOutputs.set(entryId, text);
  while (fullOutputs.size > FULL_MAX) {
    const oldest = fullOutputs.keys().next();
    if (oldest.done) break;
    fullOutputs.delete(oldest.value);
  }
}

export function cachedFullOutput(entryId: string): string | undefined {
  const text = fullOutputs.get(entryId);
  if (text !== undefined) {
    fullOutputs.delete(entryId);
    fullOutputs.set(entryId, text);
  }
  return text;
}

export function clearFullOutputs(): void {
  fullOutputs.clear();
}

/** The result entry's text content (`textOf` on its parts). */
export const resultTextOf = (entry: SessionEntry | undefined): string => {
  const content = entry?.message?.content;
  if (typeof content === 'string') return content;
  if (!Array.isArray(content)) return '';
  return content
    .filter((p) => p.type === 'text' || typeof p.text === 'string')
    .map((p) => String((p as { text?: unknown }).text ?? ''))
    .join('\n');
};
