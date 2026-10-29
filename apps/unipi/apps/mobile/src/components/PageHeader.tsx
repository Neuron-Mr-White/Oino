/**
 * The one page header (UNI-218 F1, DESIGN.md §1.6): every top-level screen
 * (Sessions, Chat, Board, Files, Preview, Settings) uses it, so the title
 * sits at the same place on all of them.
 *
 *   [lead: PC chip / back]            [tools…]
 *   Title (30/36 600)
 *   meta line (13px muted)
 *
 * On the desktop the lead row is always there (44px), so switching screens
 * never moves the title. On the phone a header without `lead` puts its
 * tools on the title row (the compact look Sessions/Files/Settings had).
 * The phone keeps 52px free on the right for the floating needs-you bell;
 * the desktop renders the bell inside `tools` instead (`bell`).
 */
import { Show, type JSX } from 'solid-js';
import { layout } from '../lib/layout';
import { can } from '../lib/platform';
import { NeedsYouBell } from './NeedsYouBell';
import { setNeedsYouOpen } from './FloatingBell';

export function PageHeader(props: {
  title: JSX.Element;
  /** Left of the top row: a PC chip, a back button… */
  lead?: JSX.Element;
  /** Right of the top row: icon buttons. */
  tools?: JSX.Element;
  /** Line under the title. */
  meta?: JSX.Element;
  /** Replaces the plain title text (e.g. the board's project picker button). */
  titleSlot?: JSX.Element;
  class?: string;
  /** Bell in the header's tools on the desktop shell (default on). */
  bell?: boolean;
  testId?: string;
}) {
  const desktop = () => layout().desktop();
  return (
    <header
      class={`page-head ${props.class ?? ''}`}
      data-lead={props.lead ? '' : undefined}
      data-testid={props.testId ?? 'page-head'}
    >
      <div class="page-head-lead">{props.lead}</div>
      <div class="page-head-tools">
        {props.tools}
        <Show when={desktop() && props.bell !== false && can('hosts')}>
          <NeedsYouBell onOpen={() => setNeedsYouOpen(true)} />
        </Show>
      </div>
      <div class="page-head-title">
        <Show when={props.titleSlot} fallback={<h1 class="large-title">{props.title}</h1>}>
          {props.titleSlot}
        </Show>
      </div>
      <Show when={props.meta !== undefined}>
        <p class="page-head-meta">{props.meta}</p>
      </Show>
    </header>
  );
}

