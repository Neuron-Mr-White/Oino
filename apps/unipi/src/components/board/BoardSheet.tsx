/**
 * Board bottom sheet (UNI-130), re-based on the phone `SheetFrame` (phone spec
 * §2.12; API unchanged): a Corvu drawer in the app's `.sheet` look that rises
 * with the soft keyboard (`--kb`), closes on Android Back (pushBack) and swipe
 * down. `onClose` is a *request*: a guarded sheet (a dirty New task form) can
 * refuse by not flipping `open`. It keeps its title-row ✕ on both shells; new
 * code uses `Sheet` from components/phone.
 */
import type { JSX } from 'solid-js';
import { SheetFrame } from '../phone/Sheet';

export function BoardSheet(props: {
  open: boolean;
  onClose: () => void;
  /** Accessible name (also the visible title unless `title` is given). */
  label: string;
  title?: JSX.Element;
  /** Right side of the title row (e.g. a Clear button); a close X otherwise. */
  headerAction?: JSX.Element;
  class?: string;
  /** Sticky bottom row (primary action) that stays above the keyboard. */
  footer?: JSX.Element;
  children: JSX.Element;
  testId?: string;
  /** Drag-to-dismiss off (a dirty form): Back / ✕ / outside tap still ask via onClose. */
  guarded?: boolean;
}) {
  return (
    <SheetFrame
      open={props.open}
      onClose={() => props.onClose()}
      label={props.label}
      title={props.title}
      headerAction={props.headerAction}
      class={props.class}
      footer={props.footer}
      testId={props.testId}
      noDrag={props.guarded}
      classicHead
    >
      {props.children}
    </SheetFrame>
  );
}
