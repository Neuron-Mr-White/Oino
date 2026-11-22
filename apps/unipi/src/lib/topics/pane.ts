import { createEffect, createSignal, onCleanup, onMount } from 'solid-js';
import { topicsFor } from './client';
import { fallbackPoll } from './poll-allowlist';

/** Pane bodies stay mounted when their tabs are hidden. Observe the actual body,
 * not just document visibility. No timer or subscription survives a hidden body.
 */
export function paneVisible(body: () => HTMLElement | undefined) {
  const [visible, setVisible] = createSignal(false);
  onMount(() => {
    const element = body();
    const update = () => setVisible(!document.hidden && !element?.hidden);
    const observer = new MutationObserver(update);
    if (element) observer.observe(element, { attributes: true, attributeFilter: ['hidden'] });
    document.addEventListener('visibilitychange', update);
    update();
    onCleanup(() => { observer.disconnect(); document.removeEventListener('visibilitychange', update); });
  });
  return visible;
}

export function useTopicRefresh(options: {
  host: () => string; topic: () => string; enabled: () => boolean;
  visible?: () => boolean; refresh: () => void; period: number;
}) {
  createEffect(() => {
    if (!options.enabled() || options.visible?.() === false) return;
    const stop = topicsFor(options.host()).watch(options.topic(), options.refresh,
      () => fallbackPoll(options.refresh, options.period));
    onCleanup(stop);
  });
}
