/**
 * The board's status + priority glyphs (copied from the kanboard web UI's
 * icons.tsx, UNI-130). Colour comes from CSS (`.status-<id>`,
 * `.prio-<id>` in screens/board.css, Unicrab palette).
 */
import type { JSX } from 'solid-js';

const C = 7;
const R = 5.6;

/** A pie wedge from 12 o'clock clockwise, radius r, 0 < p < 1. */
function wedge(r: number, p: number): string {
  const angle = 2 * Math.PI * p;
  const x = C + r * Math.sin(angle);
  const y = C - r * Math.cos(angle);
  return `M${C},${C} L${C},${C - r} A${r},${r} 0 ${p > 0.5 ? 1 : 0},1 ${x.toFixed(3)},${y.toFixed(3)} Z`;
}

export function StatusGlyph(props: { status: string; size?: number; class?: string }): JSX.Element {
  const size = () => props.size ?? 14;
  const body = (): JSX.Element => {
    switch (props.status) {
      case 'backlog':
        return <circle cx={C} cy={C} r={R} fill="none" stroke="currentColor" stroke-width="1.5" stroke-dasharray="1.4 1.75" />;
      case 'todo':
        return <circle cx={C} cy={C} r={R} fill="none" stroke="currentColor" stroke-width="1.5" />;
      case 'in_progress':
        return (
          <>
            <circle cx={C} cy={C} r={R} fill="none" stroke="currentColor" stroke-width="1.5" />
            <path d={wedge(3.1, 0.5)} fill="currentColor" />
          </>
        );
      case 'in_review':
        return (
          <>
            <circle cx={C} cy={C} r={R} fill="none" stroke="currentColor" stroke-width="1.5" />
            <path d={wedge(3.1, 0.75)} fill="currentColor" />
          </>
        );
      case 'blocked':
        return (
          <>
            <circle cx={C} cy={C} r={R} fill="none" stroke="currentColor" stroke-width="1.5" />
            <rect x="3.9" y="6.2" width="6.2" height="1.6" rx=".8" fill="currentColor" />
          </>
        );
      case 'done':
        return (
          <>
            <circle cx={C} cy={C} r="6.35" fill="currentColor" />
            <path d="m4.4 7.2 1.8 1.8 3.5-3.7" fill="none" stroke="var(--glyph-knockout)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
          </>
        );
      case 'cancelled':
        return (
          <>
            <circle cx={C} cy={C} r="6.35" fill="currentColor" />
            <path d="m5 5 4 4M9 5 5 9" fill="none" stroke="var(--glyph-knockout)" stroke-width="1.5" stroke-linecap="round" />
          </>
        );
      default:
        return (
          <>
            <rect x="1.5" y="3" width="11" height="3" rx="1" fill="none" stroke="currentColor" stroke-width="1.3" />
            <path d="M2.5 6v4.5a1 1 0 0 0 1 1h7a1 1 0 0 0 1-1V6" fill="none" stroke="currentColor" stroke-width="1.3" />
          </>
        );
    }
  };
  return (
    <svg
      width={size()}
      height={size()}
      viewBox="0 0 14 14"
      aria-hidden="true"
      class={`glyph status-${props.status}${props.class ? ` ${props.class}` : ''}`}
    >
      {body()}
    </svg>
  );
}

/** Priority: signal bars (low 1, medium 2, high 3), urgent = filled square with "!". */
export function PriorityGlyph(props: { priority: string; size?: number }): JSX.Element {
  const size = () => props.size ?? 14;
  const level = (): number => ({ low: 1, medium: 2, high: 3 } as Record<string, number>)[props.priority] ?? 0;
  return (
    <svg width={size()} height={size()} viewBox="0 0 14 14" aria-hidden="true" class={`glyph prio-${props.priority || 'none'}`}>
      {props.priority === 'urgent' ? (
        <>
          <rect x="1" y="1" width="12" height="12" rx="3" fill="currentColor" />
          <path d="M7 3.8v4" stroke="var(--glyph-knockout)" stroke-width="1.7" stroke-linecap="round" />
          <circle cx="7" cy="10.1" r=".95" fill="var(--glyph-knockout)" />
        </>
      ) : props.priority === 'none' || !props.priority ? (
        <path d="M3 7h8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" opacity=".7" />
      ) : (
        <>
          <rect x="2" y="8" width="2.4" height="4" rx=".7" fill="currentColor" opacity={level() >= 1 ? 1 : 0.25} />
          <rect x="5.8" y="5" width="2.4" height="7" rx=".7" fill="currentColor" opacity={level() >= 2 ? 1 : 0.25} />
          <rect x="9.6" y="2" width="2.4" height="10" rx=".7" fill="currentColor" opacity={level() >= 3 ? 1 : 0.25} />
        </>
      )}
    </svg>
  );
}

/** Project avatar: a tinted rounded square with the first letter. */
export function ProjectTile(props: { name: string; size?: number; hue: string; running?: boolean }): JSX.Element {
  const size = () => props.size ?? 28;
  return (
    <span
      class="project-tile"
      classList={{ 'is-running': !!props.running }}
      style={{
        width: `${size()}px`,
        height: `${size()}px`,
        '--tile-hue': props.hue,
        'font-size': `${Math.round(size() * 0.5)}px`,
        'border-radius': `${Math.round(size() * 0.3)}px`,
      }}
      aria-hidden="true"
    >
      {props.name.trim().charAt(0).toUpperCase() || '?'}
    </span>
  );
}
