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
import {
  USAGE_PALETTE,
  USAGE_RANGES,
  barShares,
  cacheHitRate,
  compactTokens,
  costNote,
  costText,
  dayLabel,
  durationText,
  fetchUsage,
  heatmapColumns,
  heatmapMonths,
  lifetimeSummary,
  modelShares,
  peakIndex,
  projectLabel,
  trendSeries,
  type HeatMode,
  type UsageRange,
} from '../../lib/usage';
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

/** ZCode's lifetime strip: total tokens, peak day, longest session, current / longest streak (366 days). */
function LifetimeStrip(props: { data: UsageData }) {
  const l = createMemo(() => lifetimeSummary(props.data.daily));
  const cell = (label: string, value: string, title?: string) => (
    <div class="usage-life-cell" title={title}>
      <span class="usage-life-value">{value}</span>
      <span class="usage-life-label">{label}</span>
    </div>
  );
  return (
    <div class="usage-life" data-testid="usage-lifetime">
      {cell('Total tokens', compactTokens(l().total), 'Last 12 months')}
      {cell('Peak tokens', compactTokens(l().peakTokens), l().peakDay ? dayLabel(l().peakDay!, true) : undefined)}
      {cell('Longest session', durationText(props.data.totals.longest_session_ms ?? 0))}
      {cell('Current streak', `${l().currentStreak} d`)}
      {cell('Longest streak', `${l().longestStreak} d`)}
    </div>
  );
}

const HEAT_MODES: Array<{ id: HeatMode; label: string }> = [
  { id: 'daily', label: 'Daily' },
  { id: 'weekly', label: 'Weekly' },
  { id: 'cumulative', label: 'Cumulative' },
];

/** The 52-week token activity heatmap with Daily / Weekly / Cumulative views. */
function Heatmap(props: { data: UsageData }) {
  const [mode, setMode] = createSignal<HeatMode>('daily');
  const cols = createMemo(() => heatmapColumns(props.data.daily, mode()));
  const months = createMemo(() => heatmapMonths(cols()));
  const tip = (c: { day: string; tokens: number; turns: number }, col: { total: number; turns: number; cells: { day: string }[] }) =>
    mode() === 'daily'
      ? `${dayLabel(c.day, true)}\n${compactTokens(c.tokens)} tokens · ${c.turns} replies`
      : `${mode() === 'weekly' ? 'Week of' : 'Through week of'} ${dayLabel(col.cells[0]!.day)}\n${compactTokens(col.total)} tokens${mode() === 'weekly' ? ` · ${col.turns} replies` : ''}`;
  return (
    <div class="usage-heat" data-testid="usage-heatmap">
      <div class="usage-heat-head">
        <h3 class="usage-h">Token activity</h3>
        <div class="usage-range" role="radiogroup" aria-label="Heatmap view">
          <For each={HEAT_MODES}>
            {(m) => (
              <button type="button" role="radio" aria-checked={mode() === m.id} class="usage-range-btn" data-testid={`usage-heat-${m.id}`} onClick={() => setMode(m.id)}>
                {m.label}
              </button>
            )}
          </For>
        </div>
      </div>
      <div class="usage-heat-grid" style={{ 'grid-template-columns': `repeat(${cols().length}, minmax(0, 1fr))` }}>
        <For each={cols()}>
          {(col) => (
            <div class="usage-heat-col">
              <For each={col.cells}>
                {(c) => <span class="usage-heat-cell" data-level={c.level} data-future={c.future ? '' : undefined} title={c.future ? undefined : tip(c, col)} />}
              </For>
            </div>
          )}
        </For>
      </div>
      <div class="usage-heat-months" style={{ 'grid-template-columns': `repeat(${cols().length}, minmax(0, 1fr))` }}>
        <For each={months()}>{(m) => <span style={{ 'grid-column': `span ${m.span}` }}>{m.span >= 2 ? m.label : ''}</span>}</For>
      </div>
      <div class="usage-heat-legend" aria-hidden="true">
        <span>Less</span>
        <For each={[0, 1, 2, 3, 4]}>{(l) => <span class="usage-heat-cell" data-level={l} />}</For>
        <span>More</span>
      </div>
    </div>
  );
}

/** Daily tokens per model (stacked bars, ZCode's daily model trend). */
function ModelTrend(props: { data: UsageData }) {
  const series = createMemo(() => trendSeries(props.data.daily));
  const color = (name: string) => (name === 'other' ? USAGE_PALETTE[6]! : USAGE_PALETTE[series().indexOf(name) % 6]!);
  const peak = createMemo(() => Math.max(0, ...props.data.daily.map((d) => d.tokens)));
  const [hover, setHover] = createSignal<number | undefined>();
  const day = () => (hover() === undefined ? undefined : props.data.daily[hover()!]);
  const gap = () => (props.data.daily.length <= 14 ? 6 : props.data.daily.length <= 40 ? 3 : 1);
  return (
    <div class="usage-chart" data-testid="usage-trend">
      <h3 class="usage-h">Daily token trend</h3>
      <div class="usage-legend">
        <For each={series()}>{(n) => <span class="usage-legend-item"><i style={{ background: color(n) }} />{n === 'other' ? 'Other models' : n}</span>}</For>
      </div>
      <div class="usage-chart-read" aria-live="polite">
        <Show when={day()} fallback={<span class="usage-dim">Hover a day for its models</span>}>
          {(d) => (
            <>
              <span class="usage-chart-day">{dayLabel(d().day, true)}</span>
              <span class="usage-chart-tokens">{compactTokens(d().tokens)} tokens</span>
              <For each={series().filter((n) => (d().models?.[n] ?? 0) > 0)}>
                {(n) => <span class="usage-dim"><i class="usage-dot" style={{ background: color(n) }} />{compactTokens(d().models![n]!)}</span>}
              </For>
            </>
          )}
        </Show>
      </div>
      <div class="usage-bars" style={{ gap: `${gap()}px` }} onMouseLeave={() => setHover(undefined)}>
        <For each={props.data.daily}>
          {(d, i) => (
            <div class="usage-bar-cell usage-stack" tabindex={0} data-active={hover() === i() ? '' : undefined} onMouseEnter={() => setHover(i())} onFocus={() => setHover(i())} data-testid="usage-bar">
              <div class="usage-stack-col" style={{ height: `${peak() > 0 ? Math.max((d.tokens / peak()) * 100, d.tokens > 0 ? 3 : 0) : 0}%` }}>
                <For each={series()}>
                  {(n) => <span style={{ 'flex-grow': String(d.models?.[n] ?? 0), background: color(n) }} />}
                </For>
                <Show when={!Object.keys(d.models ?? {}).length && d.tokens > 0}>
                  <span style={{ 'flex-grow': '1', background: USAGE_PALETTE[0] }} />
                </Show>
              </div>
            </div>
          )}
        </For>
      </div>
      <div class="usage-axis" aria-hidden="true">
        <span>{props.data.daily[0] ? dayLabel(props.data.daily[0].day) : ''}</span>
        <span>{props.data.daily.length ? dayLabel(props.data.daily[props.data.daily.length - 1]!.day) : ''}</span>
      </div>
    </div>
  );
}

/** Model usage donut + legend (ZCode's model usage pie). */
function ModelDonut(props: { data: UsageData }) {
  const rows = createMemo(() => modelShares(props.data.by_model));
  const total = () => rows().reduce((s, r) => s + r.tokens, 0);
  const arcs = createMemo(() => {
    let at = 0;
    return rows().map((r, i) => {
      const from = at;
      at += r.share;
      return { ...r, from, to: at, color: USAGE_PALETTE[i % USAGE_PALETTE.length]! };
    });
  });
  const R = 42;
  const C = 2 * Math.PI * R;
  return (
    <div class="usage-donut-wrap" data-testid="usage-donut">
      <h3 class="usage-h">Model usage</h3>
      <Show when={rows().length > 0} fallback={<p class="usage-dim usage-empty-note">Nothing in this range.</p>}>
        <div class="usage-donut">
          <svg viewBox="0 0 100 100" class="usage-donut-svg" role="img" aria-label="Model share">
            <circle cx="50" cy="50" r={R} fill="none" stroke="var(--color-surface-3)" stroke-width="12" />
            <For each={arcs()}>
              {(a) => (
                <circle cx="50" cy="50" r={R} fill="none" stroke={a.color} stroke-width="12" stroke-dasharray={`${Math.max(0, (a.to - a.from) * C - 1)} ${C}`} stroke-dashoffset={String(-a.from * C)} transform="rotate(-90 50 50)">
                  <title>{`${a.name}: ${compactTokens(a.tokens)} tokens (${Math.round(a.share * 100)}%)`}</title>
                </circle>
              )}
            </For>
            <text x="50" y="49" text-anchor="middle" class="usage-donut-num">{compactTokens(total())}</text>
            <text x="50" y="61" text-anchor="middle" class="usage-donut-sub">tokens</text>
          </svg>
          <ul class="usage-donut-list">
            <For each={arcs()}>
              {(a) => (
                <li>
                  <i style={{ background: a.color }} />
                  <span class="usage-donut-name" title={a.name}>{a.name}</span>
                  <span class="usage-donut-share">{Math.round(a.share * 1000) / 10}%</span>
                  <span class="usage-donut-tok">{compactTokens(a.tokens)}</span>
                </li>
              )}
            </For>
          </ul>
        </div>
      </Show>
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
      <ModelTrend data={props.data} />
      <div class="usage-breaks">
        <ModelDonut data={props.data} />
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
  // The lifetime strip + heatmap: one 366-day read (cached host-side per file).
  const [year, { refetch: refetchYear }] = createResource(() => settingsHostId(), (host) => fetchUsage(host, 366));
  const yearData = () => {
    const r = year.latest;
    return r && r.ok ? r.value : undefined;
  };
  return (
    <Section id="sec-usage" title="Usage" icon={<BarChart3 size={14} />} intro="Tokens and cost across every session on this PC, read from the session files.">
      <div class="usage" data-testid="usage-page">
        <Show when={yearData()}>
          {(y) => (
            <>
              <LifetimeStrip data={y()} />
              <Heatmap data={y()} />
            </>
          )}
        </Show>
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
          <button type="button" class="usage-refresh" onClick={() => (void refetch(), void refetchYear())} aria-label="Refresh usage" title="Refresh" disabled={data.loading} data-testid="usage-refresh">
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
