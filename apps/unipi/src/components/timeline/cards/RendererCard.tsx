/**
 * Extension-rendered card (WP-A8, G-21): `custom` / `custom_message`
 * entries render through the owner's `_oino/render` spec (target
 * `customEntry` / `customMessage`, key = `customType`) as a U9 `NodeView`
 * — replacing unipi 3.x's hard-coded content types. While the answer is
 * in flight (or when the owner answers `null` / times out at 2 s) the
 * frontend's own light row shows: `details.unipiHarness` source · title,
 * else the entry's title/text/command.
 */
import { openBgTaskTab } from '../../panes/tabs/sessionTabs';
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
  const source = str(harness.source) ?? str(data.source) ?? 'Unipi';
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

/** Parses `<background-task-notification>` XML / details into a clean native completion card (D-06: no raw temp path or 3-col WASM row). */
function parseBgNotification(entry: SessionEntry): { name: string; status: 'completed' | 'failed' | 'killed' | 'running'; meta: string; id?: string; error?: string } | undefined {
  if (entry.customType !== 'background-task-notification') return undefined;
  const rawContent = (entry as { content?: unknown }).content ?? entry.message?.content;
  const text = typeof rawContent === 'string' ? rawContent : '';
  const details = (entry.message?.details ?? entry.details ?? {}) as Record<string, unknown>;
  if (!text.includes('<background-task-notification>') && typeof details.taskId !== 'string') return undefined;
  const tag = (name: string) => {
    const m = new RegExp(`<${name}>([\\s\\S]*?)</${name}>`).exec(text);
    return m?.[1]?.trim().replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&amp;/g, '&');
  };
  const rawStatus = (tag('status') ?? str(details.status) ?? 'completed').toLowerCase();
  const status = rawStatus === 'failed' || rawStatus === 'error' ? 'failed' : rawStatus === 'killed' || rawStatus === 'stopped' ? 'killed' : rawStatus === 'running' ? 'running' : 'completed';
  const name = tag('task-name') ?? str(details.name) ?? str(details.taskId) ?? 'Background task';
  const exit = tag('exit-code');
  const id = tag('task-id') ?? str(details.taskId);
  const error = tag('error') ?? str(details.error);
  const metaParts: string[] = [];
  if (status === 'killed') metaParts.push('stopped');
  else if (exit && exit !== 'null') metaParts.push(`exit ${exit}`);
  else if (status === 'failed') metaParts.push('failed');
  if (id && id !== name) metaParts.push(id);
  return { name, status, meta: metaParts.join(' · '), ...(id ? { id } : {}), ...(error ? { error } : {}) };
}

export function RendererCard(props: { entry: SessionEntry; ctx: TimelineCtx }) {
  const e = () => props.entry;
  const hidden = () => e().type === 'custom_message' && ((e() as { display?: unknown }).display === false || (e().message as { display?: unknown } | undefined)?.display === false);
  const bgNotif = () => parseBgNotification(e());
  const target = () => cardTargetOf(e());
  const key = () => e().customType ?? '';
  const node = () => {
    if (hidden() || bgNotif() || !target() || !key()) return undefined;
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
        when={!bgNotif()}
        fallback={
          <div class="tl-cardwrap" data-custom-type={key() || undefined} data-testid="tl-card">
            <BgNotificationCard info={bgNotif()!} ctx={props.ctx} />
          </div>
        }
      >
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
    </Show>
  );
}

function BgNotificationCard(props: { info: NonNullable<ReturnType<typeof parseBgNotification>>; ctx: TimelineCtx }) {
  const open = () => props.info.id && openBgTaskTab({ hostId: props.ctx.session.hostId, sessionId: props.ctx.session.sessionId() }, { taskId: props.info.id, name: props.info.name });
  const badge = () => (props.info.status === 'failed' ? 'Failed' : props.info.status === 'killed' ? 'Stopped' : props.info.status === 'running' ? 'Background' : 'Done');
  return (
    <div class="tool-bg tl-bg-card" data-status={props.info.status} data-testid="tl-bg-notification">
      <div
        class="tool-bg-head"
        data-border={props.info.error ? '' : undefined}
        {...(props.info.id ? { role: 'button' as const, tabindex: 0, title: 'Open live output', 'data-openable': '', onClick: open, onKeyDown: (e: KeyboardEvent) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), open()) } : {})}
      >
        <span class="tool-bg-badge" data-status={props.info.status}>
          {badge()}
        </span>
        <span class="tool-bg-title">{props.info.name}</span>
        <Show when={props.info.meta}>
          <span class="tool-bg-meta">{props.info.meta}</span>
        </Show>
      </div>
      <Show when={props.info.error}>
        <pre class="tool-term-out" data-error="">
          {props.info.error}
        </pre>
      </Show>
    </div>
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
