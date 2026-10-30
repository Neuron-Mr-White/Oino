/**
 * Tool row (WP-A8, Z-37): unipi's anatomy — icon by category, tool name
 * (shimmer while busy), subject (verified path / command / query), `+N −M`,
 * the error's first line in red — plus ZCode's **hover-only chevron**
 * (visible on hover when done; always while running or open). The body
 * mounts only while open (`expansion` keeps the state across remounts).
 */
import { Show } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import { ChevronRight } from 'lucide-solid';
import { diffStat, toolCategory, toolSubject } from '../../../lib/chat/model';
import { createPresence } from '../../../lib/motion';
import { expansionOpen, toggleExpansion } from '../../../lib/timeline/expansion';
import type { ToolRun } from '../../../lib/session/turns';
import type { TimelineCtx } from '../context';
import { TOOL_ICON } from './toolIcons';
import { ToolBody } from './ToolBody';

export function ToolRow(props: { tool: ToolRun; ctx: TimelineCtx; itemKey: string }) {
  const t = () => props.tool;
  const open = () => expansionOpen(props.ctx.sessionId(), `tool:${props.itemKey}`, false);
  const toggle = () => toggleExpansion(props.ctx.sessionId(), `tool:${props.itemKey}`, false);
  const cat = () => toolCategory(t().name);
  const busy = () => t().state === 'running' || t().state === 'preparing';
  const errLine = () => (t().state === 'error' ? (t().output ?? '').split('\n').find((l) => l.trim()) : undefined);
  const stat = () => diffStat({ name: t().name, args: t().args });
  const subject = () => errLine() ?? (t().state === 'preparing' ? 'preparing…' : toolSubject({ name: t().name, args: t().args }, props.ctx.cwd()) || t().name);
  // Z-91: the body fades out (160 ms) when the row folds instead of vanishing.
  const body = createPresence(open);
  return (
    <div class="tool tl-tool" data-state={t().state}>
      <button
        type="button"
        class="tool-row"
        data-testid="tl-tool-row"
        aria-expanded={open()}
        disabled={t().state === 'preparing'}
        onClick={toggle}
        title={t().name}
      >
        <span class="tool-icon">
          <Dynamic component={TOOL_ICON[cat()]} size={15} />
        </span>
        <span class="tool-name" classList={{ shimmer: busy() }}>
          {t().name}
        </span>
        <span class="tool-sep" />
        <span class="tool-subject" data-error={errLine() ? '' : undefined}>
          {subject()}
        </span>
        <Show when={stat()}>
          <span class="tool-stat">
            <span class="add">+{stat()!.add}</span> <span class="del">−{stat()!.del}</span>
          </span>
        </Show>
        {/* Z-37: the chevron shows on hover (always on phone / when open) */}
        <ChevronRight size={15} class="proc-chev" data-open={open() ? '' : undefined} />
      </button>
      <Show when={body.mounted()}>
        <ToolBody tool={t()} ctx={props.ctx} leaving={body.leaving()} />
      </Show>
    </div>
  );
}
