/**
 * Tool row (WP-A8, Z-37): unipi's anatomy — icon by category, tool name
 * (shimmer while busy), subject (verified path / command / query), `+N −M`,
 * duration, status, and the error's first line in red — plus ZCode's
 * **hover-only chevron** (visible on hover when done; always while running
 * or open). The body mounts only while open (`expansion` keeps the state
 * across remounts).
 */
import { Show } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import { ChevronRight, CircleAlert } from 'lucide-solid';
import { diffStat, formatDuration, toolCategory, toolSubject } from '../../../lib/chat/model';
import { createPresence } from '../../../lib/motion';
import { expansionOpen, toggleExpansion } from '../../../lib/timeline/expansion';
import type { ToolRun } from '../../../lib/session/turns';
import type { TimelineCtx } from '../context';
import { TOOL_ICON } from './toolIcons';
import { ToolBody } from './ToolBody';

function fallbackTarget(args: Record<string, unknown>, cwd?: string): string {
  for (const key of ['cmd', 'script', 'dir', 'directory', 'target', 'selector', 'ref', 'profile', 'subagent_type', 'action', 'taskId', 'name', 'id']) {
    const v = args[key];
    if (typeof v === 'string' && v.trim()) {
      let s = v.trim().split('\n')[0]!;
      if (cwd && s.startsWith(cwd + '/')) s = s.slice(cwd.length + 1);
      return s.replace(/^\/home\/[^/]+\//, '~/');
    }
  }
  for (const [, v] of Object.entries(args)) {
    if (typeof v === 'string' && v.trim()) return v.trim().split('\n')[0]!.slice(0, 120);
    if (typeof v === 'number' || typeof v === 'boolean') return String(v);
  }
  return '';
}

export function formatToolDuration(ms: number): string {
  if (ms < 1000) return `${Math.max(1, Math.round(ms))}ms`;
  if (ms < 10_000) return `${(ms / 1000).toFixed(1).replace(/\.0$/, '')}s`;
  return formatDuration(ms);
}

export function ToolRow(props: { tool: ToolRun; ctx: TimelineCtx; itemKey: string }) {
  const t = () => props.tool;
  const open = () => expansionOpen(props.ctx.sessionId(), `tool:${props.itemKey}`, false);
  const toggle = () => toggleExpansion(props.ctx.sessionId(), `tool:${props.itemKey}`, false);
  const cat = () => toolCategory(t().name);
  const busy = () => t().state === 'running' || t().state === 'preparing';
  const errLine = () => (t().state === 'error' ? (t().output ?? '').split('\n').map((l) => l.trim()).find(Boolean) : undefined);
  const stat = () => diffStat({ name: t().name, args: t().args });
  const target = () => toolSubject({ name: t().name, args: t().args }, props.ctx.cwd()) || fallbackTarget(t().args, props.ctx.cwd());
  const subject = () => errLine() ?? (t().state === 'preparing' ? 'preparing…' : target());
  const dur = () => {
    const ms = t().durationMs;
    return !busy() && ms !== undefined && Number.isFinite(ms) && ms >= 50 ? formatToolDuration(ms) : undefined;
  };
  const statusBadge = () => {
    if (t().state === 'error' && !errLine()) return 'failed';
    if (t().state === 'stopped') return 'stopped';
    return undefined;
  };
  const fullTitle = () => [t().name, subject(), statusBadge(), dur()].filter(Boolean).join(' · ');
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
        title={fullTitle()}
      >
        <span class="tool-icon">
          <Show when={t().state === 'error'} fallback={<Dynamic component={TOOL_ICON[cat()]} size={15} />}>
            <CircleAlert size={15} />
          </Show>
        </span>
        <span class="tool-name" classList={{ shimmer: busy() }}>
          {t().name}
        </span>
        <Show when={subject()}>
          <span class="tool-sep" />
          <span class="tool-subject" data-error={errLine() ? '' : undefined}>
            {subject()}
          </span>
        </Show>
        <Show when={statusBadge()}>
          <span class="tool-status" data-state={t().state}>
            {statusBadge()}
          </span>
        </Show>
        <Show when={stat()}>
          <span class="tool-stat">
            <span class="add">+{stat()!.add}</span> <span class="del">−{stat()!.del}</span>
          </span>
        </Show>
        <Show when={dur()}>
          <span class="tool-dur" data-testid="tl-tool-dur">
            {dur()}
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
