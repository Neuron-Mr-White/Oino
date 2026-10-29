/**
 * Capability gate (UNI-117, lib/platform.ts). Renders `children` only when
 * every capability in `cap` is available here, otherwise `fallback` — by
 * default the standard <UnavailableNotice>.
 *
 *   <Requires cap="chat"><ChatScreen /></Requires>
 *   <Requires cap={['files', 'blob']} fallback={<MyOwnNotice />}>…</Requires>
 *   <Requires cap="camera" fallback={null}><ScanButton /></Requires>   // just hide
 */
import { Show, type JSX } from 'solid-js';
import { Globe, Monitor, Smartphone, Clock, ExternalLink } from 'lucide-solid';
import { can, unavailable, type Capability, type UnavailableKind } from '../lib/platform';
import './Requires.css';

/** Where to get the app (the web notice links here). */
export const APP_SITE = 'https://unipi.nrn.one';

export function Requires(props: {
  cap: Capability | Capability[];
  /** What to show instead; default: <UnavailableNotice cap=… />. `null` hides. */
  fallback?: JSX.Element | null;
  /** Notice layout for the default fallback. */
  variant?: 'page' | 'inline';
  children: JSX.Element;
}) {
  const caps = () => (Array.isArray(props.cap) ? props.cap : [props.cap]);
  const missing = () => caps().find((cap) => !can(cap));
  return (
    <Show
      when={!missing()}
      fallback={props.fallback === undefined ? <UnavailableNotice cap={missing()!} variant={props.variant} /> : props.fallback}
    >
      {props.children}
    </Show>
  );
}

const ICON: Record<UnavailableKind, (p: { size: number }) => JSX.Element> = {
  'needs-app': (p) => <Globe size={p.size} />,
  'desktop-only': (p) => <Monitor size={p.size} />,
  'phone-only': (p) => <Smartphone size={p.size} />,
  'not-yet': (p) => <Clock size={p.size} />,
};

/**
 * The one look for "this can't work here": a tinted badge with the reason
 * ("Not available in the web version" / "Desktop only" / "Phone only"), a
 * one-line explanation, and an optional action. `page` centres it in the
 * screen (a whole tab is unavailable); `inline` is a compact card inside a
 * list or settings group.
 */
export function UnavailableNotice(props: {
  cap: Capability;
  variant?: 'page' | 'inline';
  /** Override the explanation line. */
  body?: string;
  action?: JSX.Element;
}) {
  const info = () => unavailable(props.cap) ?? { kind: 'not-yet' as const, title: 'Not available', body: '' };
  const variant = () => props.variant ?? 'page';
  return (
    <div class="platform-notice" data-variant={variant()} data-kind={info().kind} role="status" data-testid="unavailable-notice">
      <span class="platform-notice-icon" aria-hidden="true">
        {ICON[info().kind]({ size: variant() === 'page' ? 26 : 18 })}
      </span>
      <div class="platform-notice-text">
        <p class="platform-notice-title">{info().title}</p>
        <p class="platform-notice-body">{props.body ?? info().body}</p>
      </div>
      <Show
        when={props.action}
        fallback={
          <Show when={info().kind === 'needs-app' && variant() === 'page'}>
            <div class="platform-notice-action">
              <a class="platform-notice-link" href={APP_SITE} target="_blank" rel="noopener noreferrer" data-testid="get-app-link">
                Get the UniPi app <ExternalLink size={14} />
              </a>
              <a class="platform-notice-alt" href="#/board">
                Open the board
              </a>
            </div>
          </Show>
        }
      >
        <div class="platform-notice-action">{props.action}</div>
      </Show>
    </div>
  );
}
