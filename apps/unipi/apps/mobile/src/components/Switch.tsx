/** The app's one on/off switch (Settings rows, Sharing with phones). */
import { Switch as KSwitch } from '@kobalte/core/switch';

export function SwitchField(props: { checked: boolean; onChange: (v: boolean) => void; label: string; disabled?: boolean; testId?: string }) {
  return (
    <KSwitch
      checked={props.checked}
      onChange={props.onChange}
      disabled={props.disabled}
      class="inline-flex items-center data-[disabled]:opacity-50"
      aria-label={props.label}
      data-testid={props.testId}
    >
      <KSwitch.Input />
      {/* The thumb is centred on both axes: flex centring vertically
          (it used to sit at the top, UNI-128), an equal 3px inset
          horizontally in both states (44 - 20 - 3 = 21). */}
      <KSwitch.Control class="relative flex h-[26px] w-11 shrink-0 items-center rounded-pill bg-surface-3 transition-colors data-[checked]:bg-shell">
        <KSwitch.Thumb class="block h-5 w-5 shrink-0 rounded-pill bg-text translate-x-[3px] transition-transform data-[checked]:translate-x-[21px]" />
      </KSwitch.Control>
    </KSwitch>
  );
}
