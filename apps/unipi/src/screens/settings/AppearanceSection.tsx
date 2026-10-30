/**
 * Appearance section (WP-A24, Z-114): UI font size 12–20 px driving
 * `--ui-font-size`, code font size + wrap + line numbers. No light theme
 * (brand ruling); terminal and chat keep their own size settings (Terminal
 * section / markdown scale).
 */
import { Show } from 'solid-js';
import { AlignJustify, CaseSensitive, ListOrdered, Type, WrapText } from 'lucide-solid';
import { useSettings } from '../../settings/context';
import { CODE_FONT_MAX, CODE_FONT_MIN, UI_FONT_MAX, UI_FONT_MIN } from '../../settings/appearance';
import { Row, Section, Stepper } from './rows';
import { SwitchField } from '../../components/Switch';
import { can } from '../../lib/platform';

export function AppearanceSection() {
  const [settings, update] = useSettings();
  return (
    <Section title="Appearance" id="sec-appearance" icon={<Type size={14} />}>
      <Row
        label="UI font size"
        description="Text size across the app; icons and layout stay the same"
        icon={<CaseSensitive size={18} />}
      >
        <Stepper
          label="UI font size"
          value={settings.appearance.uiFontSize}
          min={UI_FONT_MIN}
          max={UI_FONT_MAX}
          step={1}
          suffix="px"
          onChange={(v) => update('appearance.uiFontSize', v)}
        />
      </Row>
      <Row label="Code font size" description="Code blocks, the file editor and diffs" icon={<AlignJustify size={18} />}>
        <Stepper
          label="Code font size"
          value={settings.appearance.codeFontSize}
          min={CODE_FONT_MIN}
          max={CODE_FONT_MAX}
          step={1}
          suffix="px"
          onChange={(v) => update('appearance.codeFontSize', v)}
        />
      </Row>
      <Row label="Wrap code lines" description="Wrap long code lines instead of scrolling sideways" icon={<WrapText size={18} />}>
        <SwitchField
          label="Wrap code lines"
          checked={settings.appearance.codeWrap}
          onChange={(v) => update('appearance.codeWrap', v)}
          testId="code-wrap"
        />
      </Row>
      <Show when={can('files')}>
        <Row label="Code line numbers" description="Line numbers in the file editor" icon={<ListOrdered size={18} />}>
          <SwitchField
            label="Code line numbers"
            checked={settings.appearance.codeLineNumbers}
            onChange={(v) => update('appearance.codeLineNumbers', v)}
            testId="code-line-numbers"
          />
        </Row>
      </Show>
    </Section>
  );
}
