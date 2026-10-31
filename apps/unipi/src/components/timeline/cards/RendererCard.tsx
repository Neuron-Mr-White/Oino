/**
 * Extension-rendered card (WP-A8, G-21): `custom` / `custom_message`
 * entries render through the owner's `_oino/render` spec (target
 * `customEntry` / `customMessage`, key = `customType`) as a U9 `NodeView`
 * — replacing unipi 3.x's hard-coded content types. While the answer is
 * in flight (or when the owner answers `null` / times out at 2 s) the
 * frontend's own light row shows: `details.unipiHarness` source · title,
 * else the entry's title/text/command.
 */
import { Show } from 'solid-js';
import { Sparkles } from 'lucide-solid';
import type { SessionEntry } from '../../../lib/session/types';
import { sessionRenderCache, widthClassOf } from '../../../lib/session/renderCache';
import { NodeView } from '../../../ui/NodeView';
import type { TimelineCtx } from '../context';

export type CardTarget = 'customMessage' | 'customEntry';

export const cardTargetOf = (entry: SessionEntry): CardTarget | undefined =>
  entry.type === 'custom_message' ? 'customMessage' : entry.type === 'custom' ? 'customEntry' : undefined;

/** The native fallback row's label: harness / provenance meta first, then entry data. */
export function nativeCardLabel(entry: SessionEntry): { source: string; title: string } {
  const m = entry.message as { unipiHarness?: { source?: unknown; title?: unknown }; oinoProvenance?: { source?: unknown; title?: unknown } } | undefined;
  const details = (entry.message?.details ?? entry.details ?? {}) as Record<string, unknown>;
  const harness = (details.unipiHarness ?? details.oinoProvenance ?? m?.unipiHarness ?? m?.oinoProvenance ?? {}) as { source?: unknown; title?: unknown; synopsis?: unknown };
  const data = (entry.data ?? {}) as Record<string, unknown>;
  const rawContent = (entry as { content?: unknown }).content ?? entry.message?.content;
  const contentText = typeof rawContent === 'string' ? rawContent : undefined;
  const source = str(harness.source) ?? str(data.source) ?? 'UniPi';
  const title =
    str(harness.title) ??
    str(harness.synopsis) ??
    str(data.title) ??
    str(data.command) ??
    str(data.text) ??
    str(contentText?.replace(/^#+\s*/, '')) ??
    entry.customType ??
    entry.type;
  return { source, title: title.split('\n')[0]!.slice(0, 120) };
}

const str = (v: unknown): string | undefined => (typeof v === 'string' && v.trim() ? v.trim() : undefined);

export function RendererCard(props: { entry: SessionEntry; ctx: TimelineCtx }) {
  const e = () => props.entry;
  const hidden = () => e().type === 'custom_message' && ((e() as { display?: unknown }).display === false || (e().message as { display?: unknown } | undefined)?.display === false);
  const target = () => cardTargetOf(e());
  const key = () => e().customType ?? '';
  const node = () => {
    if (hidden() || !target() || !key()) return undefined;
    return sessionRenderCache(props.ctx.session).node(
      { target: target()!, key: key()!, entryId: e().id, width: widthClassOf(props.ctx.columnWidth()) },
      { density: 'regular', expanded: false, data: { entry: e(), message: e().message ?? null, customType: key(), data: e().data ?? null } },
    )();
  };
  // `custom` state entries render only when an extension renderer returns a node;
  // `custom_message` (`display !== false`) falls back to `NativeCard` while in flight or when `null`.
  return (
    <Show when={!hidden()}>
      <Show
        when={e().type === 'custom'}
        fallback={
          <div class="tl-cardwrap" data-custom-type={key() || undefined} data-testid="tl-card">
            <Show when={node()} keyed fallback={<NativeCard entry={e()} />}>
              {(n) => (
                <div class="tl-card-render" data-testid="tl-card-render">
                  <NodeView node={n} path={["render"]} mode="auto" />
                </div>
              )}
            </Show>
          </div>
        }
      >
        <Show when={node()} keyed>
          {(n) => (
            <div class="tl-cardwrap" data-custom-type={key() || undefined} data-testid="tl-card">
              <div class="tl-card-render" data-testid="tl-card-render">
                <NodeView node={n} path={["render"]} mode="auto" />
              </div>
            </div>
          )}
        </Show>
      </Show>
    </Show>
  );
}

/** The frontend's own row: `Source · title` with a spark, until (or unless)
 * an extension renderer answers. */
function NativeCard(props: { entry: SessionEntry }) {
  const label = () => nativeCardLabel(props.entry);
  const isBash = () => props.entry.message?.role === 'bashExecution';
  const text = () =>
    isBash()
      ? String((props.entry.message?.content as string) ?? '')
      : label().source === label().title
        ? label().title
        : `${label().source} · ${label().title}`;
  return (
    <div class="tl-card" data-kind={props.entry.type} title={text()}>
      <span class="tool-icon">
        <Sparkles size={15} />
      </span>
      <span class="tl-card-text truncate">{text()}</span>
    </div>
  );
}
