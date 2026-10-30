/**
 * UNI-200 dev-only gallery (mascot-gallery.html): every Unicrab state at a
 * few sizes, for the docs captures (docs/m8/mascot/capture-mascot.py).
 * `?state=<s>&size=<px>` shows one big crab (the GIF frames).
 */
import { render } from 'solid-js/web';
import { For } from 'solid-js';
import { Unicrab } from '../components/Unicrab';
import { MASCOT_STATES, STATE_LABEL, type MascotState } from '../lib/unicrab';
import '../style.css';
import '../components/mascot.css';

const params = new URLSearchParams(location.search);
const one = params.get('state') as MascotState | null;
const size = Number(params.get('size') ?? 0);

function Gallery() {
  if (one)
    return (
      <div style={{ display: 'grid', 'place-items': 'center', height: '100vh', gap: '8px' }}>
        <div style={{ display: 'grid', 'justify-items': 'center', gap: '10px' }}>
          <Unicrab state={one} size={size || 240} />
          <span style={{ color: '#A39A90', font: '600 18px Inter Variable, sans-serif' }}>{STATE_LABEL[one]}</span>
        </div>
      </div>
    );
  return (
    <div style={{ display: 'flex', 'flex-wrap': 'wrap', gap: '24px', padding: '24px' }}>
      <For each={MASCOT_STATES}>
        {(s) => (
          <div data-state-card={s} style={{ display: 'grid', 'justify-items': 'center', gap: '8px', padding: '12px', background: '#141211', 'border-radius': '14px' }}>
            <Unicrab state={s} size={size || 120} />
            <div style={{ display: 'flex', gap: '10px', 'align-items': 'end' }}>
              <Unicrab state={s} size={44} />
              <Unicrab state={s} size={30} />
            </div>
            <span style={{ color: '#A39A90', font: '600 13px sans-serif' }}>{s} · {STATE_LABEL[s]}</span>
          </div>
        )}
      </For>
    </div>
  );
}

render(() => <Gallery />, document.getElementById('root')!);
