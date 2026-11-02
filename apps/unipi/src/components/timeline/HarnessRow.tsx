/**
 * Harness / command row (WP-A6, Z-36 — kept unipi): a command echo before a
 * harness user message renders as the typed `/command args` with a
 * "Source · title" chip under it; the injected body (the prompt the harness
 * actually sent, minus the user's own words) expands inline. Cron-sent
 * prompts get a "Sent by cron" line instead (handled in TurnRow).
 */
import { createSignal, For, Show } from 'solid-js';
import { ChevronDown, Sparkles, SquareSlash } from 'lucide-solid';
import { injectedBody, textOf, type Part } from '../../lib/chat/model';
import { renderBlocks } from '../../lib/chat/markdown';
import type { Turn } from '../../lib/session/turns';
import { MdBlock } from '../chat/MdBlock';
import type { TimelineCtx } from './context';

/** The echo entry right before the turn's user message (`command-entry` custom). */
function echoOf(turn: Turn): { name: string; args: string; text?: string } | undefined {
  const c = turn.command;
  if (!c) return undefined;
  return { name: c.name, args: c.args, text: `/${c.name}${c.args ? ` ${c.args}` : ''}` };
}

/** `Kanboard · Task budget` → `Kanboard · task budget`. */
function tagOf(source: string | undefined, title: string | undefined): string | undefined {
  const t = title ? title.charAt(0).toLowerCase() + title.slice(1) : undefined;
  if (source && t) return `${source} · ${t}`;
  return source || title || undefined;
}

export function HarnessRow(props: { turn: Turn; ctx: TimelineCtx }) {
  const [open, setOpen] = createSignal(false);
  const echo = () => echoOf(props.turn);
  const m = () => props.turn.user?.message;
  const harness = () => {
    const details = (m()?.details ?? {}) as { unipiHarness?: { source?: string; title?: string }; oinoProvenance?: { source?: string; title?: string } };
    return (m()?.unipiHarness ?? details.unipiHarness ?? details.oinoProvenance ?? {}) as { source?: string; title?: string };
  };
  const sent = () => textOf(m()?.content as string | Part[] | undefined);
  const body = () => {
    const e = echo();
    if (!e) return undefined;
    return injectedBody(sent(), e.args) || undefined;
  };
  const tag = () => tagOf(harness().source, harness().title);
  return (
    <div class="chat-user tl-user tl-harness" data-testid="tl-harness">
      <div class="chat-bubble tl-harness-typed">{echo()?.text ?? sent()}</div>
      <Show when={echo()}>
        <span class="ichip tl-harness-cmd">
          <SquareSlash size={13} />
          <span class="ichip-label">/{echo()!.name}</span>
        </span>
      </Show>
      <Show when={tag()}>
        <Show
          when={body()}
          fallback={
            <span class="chat-injected-chip" data-testid="tl-harness-tag">
              <Sparkles size={12} />
              <span class="truncate">{tag()}</span>
            </span>
          }
        >
          <button
            type="button"
            class="chat-injected-chip"
            data-testid="tl-harness-tag"
            aria-expanded={open()}
            title="What Unipi added to this message"
            onClick={() => setOpen(!open())}
          >
            <Sparkles size={12} />
            <span class="truncate">{tag()}</span>
            <ChevronDown size={13} class="chat-chev" data-open={open() ? '' : undefined} />
          </button>
          <Show when={open()}>
            <div class="chat-injected-body md" data-testid="tl-harness-body">
              <For each={renderBlocks(body()!, false)}>{(html) => <MdBlock html={html} chat={props.ctx.refs} />}</For>
            </div>
          </Show>
        </Show>
      </Show>
    </div>
  );
}
