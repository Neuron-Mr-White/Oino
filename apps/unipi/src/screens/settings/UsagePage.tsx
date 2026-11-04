/**
 * Settings › Usage (owner fix 2026-11-04): token and cost history across every
 * session on this PC, after ZCode's usage statistics (summary stats, activity
 * chart, model usage) in Unipi's tokens — a range switch (7 / 30 / 90 days),
 * one summary strip, a compact daily bar chart and the model and project
 * breakdowns. The host aggregates (`usage_stats`); the session's own numbers
 * stay in the session modal's Info tab.
 */
import { For, Match, Show, Switch, createMemo, createResource, createSignal } from 'solid-js';
import { BarChart3, RefreshCw } from 'lucide-solid';
import type { UsageData } from '@unipi/protocol';
import { Section } from './rows';
import { settingsHostId } from '../../settings/target';
import { USAGE_RANGES, barShares, cacheHitRate, compactTokens, costNote, costText, dayLabel, fetchUsage, peakIndex, projectLabel, type UsageRange } from '../../lib/usage';
import './usage.css';

const ROWS_SHOWN = 6;

function Stat(props: { label: string; value: string; sub?: string; testId: string; title?: string }) {
  return (
    <div class="usage-stat" data-testid={props.testId} title={props.title}>
      <span class="usage-stat-label">{props.label}</span>
      <span class="usage-stat-value">{props.value}</span>
      <Show when={props.sub}>{(sub) => <span class="usage-stat-sub">{sub()}</span>}</Show>
    </div>
  );
}

/** One daily bar per local day; hover or focus reads a day out, the readout rests on the peak. */
function DailyChart(props: { data: UsageData }) {
  const daily = () => props.data.daily;
  const shares = createMemo(() => barShares(daily().map((d) => d.tokens)));
  const peak = createMemo(() => peakIndex(daily().map((d) => d.tokens)));
  const [hover, setHover] = createSignal<number | undefined>();
  const shown = () => hover() ?? (peak() >= 0 ? peak() : undefined);
  const day = () => (shown() === undefined ? undefined : daily()[shown()!]);
  const gap = () => (daily().length <= 14 ? 6 : daily().length <= 40 ? 3 : 1);
  const axis = () => {
    const d = daily();
    if (d.length === 0) return [];
    return [d[0]!, d[Math.floor((d.length - 1) / 2)]!, d[d.length - 1]!].map((x) => dayLabel(x.day));
  };
  return (
    <div class="usage-chart" data-testid="usage-chart">
      <div class="usage-chart-read" aria-live="polite">
        <Show when={day()} fallback={<span class="usage-dim">No activity in this range</span>}>
          {(d) => (
            <>
              <span class="usage-chart-day">{dayLabel(d().day, true)}</span>
              <span class="usage-chart-tokens">{compactTokens(d().tokens)} tokens</span>
              <span class="usage-dim">{costText(d().cost, d().unpriced_tokens, d().tokens)}</span>
              <Show when={hover() === undefined}>
                <span class="usage-chip">Peak day</span>
              </Show>
            </>
          )}
        </Show>
      </div>
      <div class="usage-bars" style={{ gap: `${gap()}px` }} role="group" aria-label={`Tokens per day, last ${props.data.days} days`} onMouseLeave={() => setHover(undefined)}>
        <For each={daily()}>
          {(d, i) => (
            <div
              class="usage-bar-cell"
              tabindex={0}
              aria-label={`${dayLabel(d.day, true)}: ${compactTokens(d.tokens)} tokens`}
              data-testid="usage-bar"
              data-active={hover() === i() ? '' : undefined}
              onMouseEnter={() => setHover(i())}
              onFocus={() => setHover(i())}
              onBlur={() => setHover(undefined)}
            >
              <span class="usage-bar" data-empty={d.tokens <= 0 ? '' : undefined} style={{ height: `${Math.max(shares()[i()]! * 100, d.tokens > 0 ? 3 : 0)}%` }} />
            </div>
          )}
        </For>
      </div>
      <div class="usage-axis" aria-hidden="true">
        <For each={axis()}>{(label) => <span>{label}</span>}</For>
      </div>
    </div>
  );
}

function Breakdown(props: { title: string; testId: string; rows: Array<{ key: string; name: string; sub?: string; tokens: number; cost: number; unpriced: number; meta: string }>; total: number }) {
  const [all, setAll] = createSignal(false);
  const rows = () => (all() ? props.rows : props.rows.slice(0, ROWS_SHOWN));
  return (
    <div class="usage-break" data-testid={props.testId}>
      <h3 class="usage-h">{props.title}</h3>
      <Show when={props.rows.length > 0} fallback={<p class="usage-dim usage-empty-note">Nothing in this range.</p>}>
        <ul class="usage-list">
          <For each={rows()}>
            {(r) => (
              <li class="usage-item" title={r.sub ? `${r.sub}/${r.name}` : r.name}>
                <div class="usage-item-top">
                  <span class="usage-item-name">
                    {r.name}
                    <Show when={r.sub}><span class="usage-item-sub">{r.sub}</span></Show>
                  </span>
                  <span class="usage-item-tokens">{compactTokens(r.tokens)}</span>
                  <span class="usage-item-cost">{costText(r.cost, r.unpriced, r.tokens)}</span>
                </div>
                <div class="usage-share" aria-hidden="true"><span style={{ width: `${props.total > 0 ? Math.max((r.tokens / props.total) * 100, 1.5) : 0}%` }} /></div>
                <div class="usage-item-meta">{r.meta}</div>
              </li>
            )}
          </For>
        </ul>
        <Show when={props.rows.length > ROWS_SHOWN}>
          <button type="button" class="usage-more" onClick={() => setAll(!all())}>
            {all() ? 'Show fewer' : `Show all ${props.rows.length}`}
          </button>
        </Show>
      </Show>
    </div>
  );
}

function Loaded(props: { data: UsageData }) {
  const t = () => props.data.totals;
  const hit = () => cacheHitRate(t());
  const unit = (n: number, one: string) => `${n} ${one}${n === 1 ? '' : 's'}`;
  return (
    <>
      <div class="usage-stats" data-testid="usage-summary">
        <Stat
          testId="usage-tokens"
          label="Tokens"
          value={compactTokens(t().tokens)}
          sub={`${compactTokens(t().input)} in · ${compactTokens(t().output)} out${hit() !== undefined ? ` · ${Math.round(hit()! * 100)}% cache` : ''}`}
          title={`Input ${t().input.toLocaleString()}, output ${t().output.toLocaleString()}, cache read ${t().cache_read.toLocaleString()}, cache write ${t().cache_write.toLocaleString()}`}
        />
        <Stat testId="usage-cost" label="Cost" value={costText(t().cost, t().unpriced_tokens, t().tokens)} sub={costNote(t().cost, t().unpriced_tokens)} title="Priced models only; a model without a price adds tokens but no cost" />
        <Stat testId="usage-sessions" label="Sessions" value={String(t().sessions)} sub={unit(t().turns, 'reply').replace('replys', 'replies')} />
        <Stat testId="usage-days" label="Active days" value={`${t().active_days}`} sub={`of ${props.data.days}`} />
      </div>
      <DailyChart data={props.data} />
      <div class="usage-breaks">
        <Breakdown
          title="By model"
          testId="usage-models"
          total={t().tokens}
          rows={props.data.by_model.map((m) => ({ key: m.name, name: m.name, tokens: m.tokens, cost: m.cost, unpriced: m.unpriced_tokens, meta: unit(m.turns, 'reply').replace('replys', 'replies') }))}
        />
        <Breakdown
          title="By project"
          testId="usage-projects"
          total={t().tokens}
          rows={props.data.by_project.map((p) => {
            const l = projectLabel(p.name);
            return { key: p.name, name: l.name, sub: l.parent, tokens: p.tokens, cost: p.cost, unpriced: p.unpriced_tokens, meta: unit(p.sessions, 'session') };
          })}
        />
      </div>
    </>
  );
}

export function UsagePage() {
  const [range, setRange] = createSignal<UsageRange>(30);
  // The fetch never throws: a failure is a value, so the resource stays out of its error state.
  const [data, { refetch }] = createResource(
    () => ({ host: settingsHostId(), days: range() }),
    (k) => fetchUsage(k.host, k.days),
  );
  const result = () => data.latest;
  const loaded = () => {
    const r = result();
    return r && r.ok ? r.value : undefined;
  };
  const failure = () => {
    const r = result();
    return r && !r.ok ? r.error : undefined;
  };
  const errorText = () => {
    const e = failure();
    if (e?.code === 'E_UNSUPPORTED' || e?.code === 'E_SCHEMA') return 'This PC’s host is too old to report usage — update Unipi on it.';
    return e?.message || 'Usage could not be read.';
  };
  const empty = () => loaded()?.totals.turns === 0;
  return (
    <Section id="sec-usage" title="Usage" icon={<BarChart3 size={14} />} intro="Tokens and cost across every session on this PC, read from the session files.">
      <div class="usage" data-testid="usage-page">
        <div class="usage-bar-head">
          <div class="usage-range" role="radiogroup" aria-label="Time range">
            <For each={USAGE_RANGES}>
              {(d) => (
                <button type="button" role="radio" aria-checked={range() === d} class="usage-range-btn" data-testid={`usage-range-${d}`} onClick={() => setRange(d)}>
                  {d} days
                </button>
              )}
            </For>
          </div>
          <button type="button" class="usage-refresh" onClick={() => void refetch()} aria-label="Refresh usage" title="Refresh" disabled={data.loading} data-testid="usage-refresh">
            <RefreshCw size={14} classList={{ 'usage-spin': data.loading }} />
          </button>
        </div>
        <Switch>
          <Match when={failure()}>
            <div class="usage-state" data-testid="usage-error" role="alert">
              <p>{errorText()}</p>
              <button type="button" class="btn-secondary settings-btn" onClick={() => void refetch()}>Retry</button>
            </div>
          </Match>
          <Match when={!result()}>
            <div class="usage-skeleton" data-testid="usage-loading" aria-busy="true" aria-label="Reading session history">
              <div class="usage-sk-stats"><span /><span /><span /><span /></div>
              <div class="usage-sk-chart" />
            </div>
          </Match>
          <Match when={empty()}>
            <div class="usage-state" data-testid="usage-empty">
              <p>No usage in the last {range()} days.</p>
              <p class="usage-dim">Sessions that ran on this PC show up here once the agent has replied.</p>
            </div>
          </Match>
          <Match when={loaded()}>{(d) => <div class="usage-body" classList={{ 'usage-stale': data.loading }}><Loaded data={d()} /></div>}</Match>
        </Switch>
      </div>
    </Section>
  );
}
