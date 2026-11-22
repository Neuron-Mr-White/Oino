/**
 * The phone page frame (phone spec §1.2, §2.6): tab pages (56 px top bar
 * with the PC chip + ≤ 2 tools, a 28/34 large title that collapses into the
 * bar after 40 px of scroll, optional search) and pushed pages (56 px
 * compact header: Back · title/subtitle · ≤ 2 actions). The selected PC's
 * OfflineBanner is automatic on tab pages. `workspace` hides the tab bar.
 *
 * On the desktop shell it renders the existing PageHeader + body, so a
 * screen can use it on both shells without a fork.
 */
import { createSignal, createUniqueId, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { ArrowLeft, Loader2 } from 'lucide-solid';
import { layout } from '../../lib/layout';
import { PageHeader } from '../PageHeader';
import { useSelectedHost } from '../shell/HostSwitcher';
import { Fab, IconButton, SearchField, type FabProps } from './controls';
import { HostSheetButton, OfflineBanner } from './HostSheet';
import { haptic, setTabBarHidden } from './services';

/** Scroll distance after which the large title collapses into the top bar. */
export const COLLAPSE_AT = 40;
/** Scroll distance after which an extended FAB shrinks to its icon. */
export const FAB_SHRINK_AT = 80;
/** Pull distance that triggers a refresh. */
export const PULL_TRIGGER = 72;

export function PageScaffold(p: {
  title: JSX.Element;
  subtitle?: JSX.Element;
  titleSlot?: JSX.Element;
  back?: string | (() => void);
  pcChip?: boolean;
  tools?: JSX.Element;
  search?: { value: string; onInput: (q: string) => void; placeholder: string };
  fab?: FabProps;
  onRefresh?: () => Promise<void>;
  banner?: JSX.Element;
  workspace?: boolean;
  footer?: JSX.Element;
  testId?: string;
  children: JSX.Element;
}) {
  const navigate = useNavigate();
  const selected = useSelectedHost();
  const desktop = () => layout().desktop();
  const pushed = () => p.back !== undefined;
  const [scrolled, setScrolled] = createSignal(0);
  const [pull, setPull] = createSignal(0);
  const [refreshing, setRefreshing] = createSignal(false);
  const reason = `scaffold-${createUniqueId()}`;
  onMount(() => {
    if (p.workspace) setTabBarHidden(reason, true);
  });
  onCleanup(() => setTabBarHidden(reason, false));

  const goBack = () => {
    const b = p.back;
    if (typeof b === 'function') b();
    else if (typeof b === 'string') navigate(b, { replace: true });
  };

  // Pull-to-refresh (touch only, from the very top of the scroller).
  let scroller: HTMLDivElement | undefined;
  let pullStart: number | undefined;
  let crossed = false;
  const onTouchStart = (e: TouchEvent) => {
    if (!p.onRefresh || refreshing() || (scroller?.scrollTop ?? 0) > 0) return;
    pullStart = e.touches[0]?.clientY;
    crossed = false;
  };
  const onTouchMove = (e: TouchEvent) => {
    if (pullStart === undefined) return;
    const d = (e.touches[0]?.clientY ?? 0) - pullStart;
    if (d <= 0 || (scroller?.scrollTop ?? 0) > 0) {
      setPull(0);
      return;
    }
    const damped = Math.min(120, d * 0.5);
    setPull(damped);
    const now = damped >= PULL_TRIGGER;
    if (now !== crossed) {
      crossed = now;
      if (now) haptic('selection');
    }
  };
  const onTouchEnd = () => {
    if (pullStart === undefined) return;
    pullStart = undefined;
    const go = pull() >= PULL_TRIGGER && p.onRefresh;
    setPull(0);
    if (!go) return;
    setRefreshing(true);
    void p.onRefresh!().finally(() => setRefreshing(false));
  };

  const desktopBody = () => (
    <div class="page-screen ph-desktop-page" data-testid={p.testId}>
      <PageHeader
        title={p.title}
        titleSlot={p.titleSlot}
        meta={p.subtitle}
        tools={p.tools}
        lead={
          pushed() ? (
            <button type="button" class="ghost-icon-button" aria-label="Back" onClick={goBack}>
              <ArrowLeft size={18} />
            </button>
          ) : undefined
        }
      />
      <Show when={p.banner}>{p.banner}</Show>
      <Show when={p.search}>
        {(s) => (
          <div class="ph-desktop-search">
            <SearchField value={s().value} onInput={s().onInput} placeholder={s().placeholder} />
          </div>
        )}
      </Show>
      {p.children}
      <Show when={p.footer}>{p.footer}</Show>
    </div>
  );

  return (
    <Show when={!desktop()} fallback={desktopBody()}>
      <div
        class="ph-page"
        data-kind={pushed() ? 'pushed' : 'tab'}
        data-workspace={p.workspace ? '' : undefined}
        data-collapsed={pushed() || scrolled() > COLLAPSE_AT ? '' : undefined}
        data-testid={p.testId}
      >
        <header class="ph-top">
          <Show
            when={pushed()}
            fallback={
              <Show when={p.pcChip !== false}>
                <HostSheetButton />
              </Show>
            }
          >
            <IconButton icon={ArrowLeft} label="Back" onClick={goBack} testId="ph-back" />
          </Show>
          <div class="ph-top-title">
            <Show
              when={pushed()}
              fallback={
                <span class="ph-top-collapsed" aria-hidden={scrolled() > COLLAPSE_AT ? undefined : 'true'}>
                  {p.title}
                </span>
              }
            >
              <Show when={p.titleSlot} fallback={<h1 class="ph-top-h">{p.title}</h1>}>
                {p.titleSlot}
              </Show>
              <Show when={p.subtitle}>
                <span class="ph-top-sub">{p.subtitle}</span>
              </Show>
            </Show>
          </div>
          <Show when={p.tools}>
            <div class="ph-top-tools">{p.tools}</div>
          </Show>
        </header>
        <div
          ref={scroller}
          class="ph-scroll"
          onScroll={(e) => setScrolled(e.currentTarget.scrollTop)}
          onTouchStart={onTouchStart}
          onTouchMove={onTouchMove}
          onTouchEnd={onTouchEnd}
          onTouchCancel={onTouchEnd}
        >
          <Show when={pull() > 0 || refreshing()}>
            <div class="ph-pull" style={{ height: `${refreshing() ? 48 : pull()}px` }} aria-live="polite">
              <Loader2 size={22} class={refreshing() ? 'ph-spin' : undefined} style={{ opacity: Math.min(1, pull() / PULL_TRIGGER || 1) }} />
              <span class="sr-only">{refreshing() ? 'Refreshing' : ''}</span>
            </div>
          </Show>
          <Show when={!pushed() && selected()}>{(h) => <OfflineBanner hostId={h().id} />}</Show>
          <Show when={p.banner}>{p.banner}</Show>
          <Show when={!pushed()}>
            <div class="ph-large">
              <Show when={p.titleSlot} fallback={<h1 class="ph-large-title">{p.title}</h1>}>
                {p.titleSlot}
              </Show>
            </div>
          </Show>
          <Show when={p.search}>
            {(s) => (
              <div class="ph-page-search">
                <SearchField value={s().value} onInput={s().onInput} placeholder={s().placeholder} />
              </div>
            )}
          </Show>
          <div class="ph-body">{p.children}</div>
        </div>
        <Show when={p.footer}>
          <div class="ph-footer">{p.footer}</div>
        </Show>
        <Show when={p.fab}>
          {(f) => <Fab icon={f().icon} label={f().label} onClick={f().onClick} extended={f().extended !== false && scrolled() < FAB_SHRINK_AT} />}
        </Show>
      </div>
    </Show>
  );
}
