/**
 * Unicrab section (WP-A24; UNI-200): the mascot surfaces — floating crab
 * (phone), over other apps (Android overlay), mascot window (desktop tray).
 */
import { Show, createSignal, onCleanup, onMount } from 'solid-js';
import { Bell, Layers, PictureInPicture2, Smartphone } from 'lucide-solid';
import { Unicrab } from '../../components/Unicrab';
import { SwitchField } from '../../components/Switch';
import { Row, Section } from './rows';
import { floatingMascotOn, setFloatingMascotOn } from '../../components/FloatingBell';
import { MASCOT_WINDOW_SIZES, mascotWindowOn, mascotWindowSize, setMascotWindowSize, syncMascotWindow, type MascotWindowSize } from '../../lib/mascot';
import { overlayRunning, setOverlay } from '../../lib/mascotNative';
import { overlayBridge } from '../../lib/mascotFeed';
import { can } from '../../lib/platform';
import { desktopShell } from '../../lib/layout';

export function UnicrabSection() {
  const bridge = overlayBridge();
  const [overlayMsg, setOverlayMsg] = createSignal<string | undefined>();
  const [overlayChecked, setOverlayChecked] = createSignal(overlayRunning());
  const [windowOn, setWindowOn] = createSignal(mascotWindowOn());
  const [windowSize, setWindowSizeSig] = createSignal<MascotWindowSize>(mascotWindowSize());
  const pickSize = (size: MascotWindowSize) => {
    setMascotWindowSize(size);
    setWindowSizeSig(size);
    if (windowOn()) void syncMascotWindow(true);
  };
  // Back from the system "Display over other apps" page: finish turning it on.
  onMount(() => {
    const onVisible = () => {
      if (document.visibilityState !== 'visible' || overlayMsg() !== 'pending') return;
      if (bridge?.canDraw?.()) {
        setOverlay(true);
        setOverlayChecked(true);
        setOverlayMsg(undefined);
      } else setOverlayMsg('denied');
    };
    document.addEventListener('visibilitychange', onVisible);
    onCleanup(() => document.removeEventListener('visibilitychange', onVisible));
  });
  const toggleOverlay = (on: boolean) => {
    const step = setOverlay(on);
    setOverlayChecked(step === 'on');
    setOverlayMsg(step === 'needs-permission' ? 'pending' : undefined);
  };
  const phone = () => !desktopShell() && can('hosts');
  return (
    <Show when={phone() || !!bridge?.start || can('tray')}>
      <Section title="Unicrab" id="sec-unicrab" icon={<Bell size={14} />}>
        <div class="mascot-settings-hero" aria-hidden="true">
          <Unicrab state="idle" size={64} />
          <p class="text-xs text-text-dim">Unicrab shows what your agents are doing and pops up their replies. Tap it for everything that needs you.</p>
        </div>
        <Show when={phone()}>
          <Row label="Floating crab" description="A button you can drag to any edge; long-press for quick actions" icon={<Smartphone size={18} />}>
            <SwitchField label="Floating crab" checked={floatingMascotOn()} onChange={setFloatingMascotOn} />
          </Row>
        </Show>
        <Show when={bridge?.start}>
          <Row
            label="Over other apps"
            icon={<Layers size={18} />}
            description={
              overlayMsg() === 'pending'
                ? 'Allow "Display over other apps" for UniPi, then come back'
                : overlayMsg() === 'denied'
                  ? 'Not allowed yet: UniPi needs "Display over other apps"'
                  : 'Keeps the crab and its replies on screen while you use other apps (a small ongoing notification shows it is on)'
            }
          >
            <SwitchField label="Over other apps" checked={overlayChecked()} onChange={toggleOverlay} />
          </Row>
        </Show>
        <Show when={can('tray')}>
          <Row label="Mascot window" description="An always-on-top crab with reply bubbles; also in the tray menu" icon={<PictureInPicture2 size={18} />}>
            <SwitchField label="Mascot window" checked={windowOn()} onChange={(v) => (setWindowOn(v), void syncMascotWindow(v))} />
          </Row>
          <Show when={windowOn()}>
            <Row label="Crab size" description="Only the crab floats on your desktop (no background); drag it anywhere">
              <div class="mascot-size-options" role="radiogroup" aria-label="Crab size" data-testid="mascot-size">
                {MASCOT_WINDOW_SIZES.map((size, i) => (
                  <button type="button" role="radio" aria-checked={windowSize() === size} onClick={() => pickSize(size)}>
                    {['Small', 'Medium', 'Large'][i]}
                  </button>
                ))}
              </div>
            </Row>
          </Show>
        </Show>
      </Section>
    </Show>
  );
}
