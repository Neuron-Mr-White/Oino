/**
 * The model row's modality icons (owner: "like OpenRouter's Input Type row"):
 * a compact row of small monochrome icons for what the model **accepts**
 * (pi `input`: text, image, audio, video, file / PDF), each with a tooltip and
 * an aria-label ("Accepts images"). When the catalogue says the model
 * **produces** something other than text (image / speech models), an arrow
 * and the output icons follow. Unknown modality names are skipped.
 */
import { For, Show, type JSX } from 'solid-js';
import { ArrowRight, AudioLines, FileText, Image, Type, Video } from 'lucide-solid';
import type { ModelInfo } from '../../lib/model/picker';

export type Modality = 'text' | 'image' | 'audio' | 'video' | 'file';

const ORDER: Modality[] = ['text', 'image', 'audio', 'video', 'file'];

/** A catalogue modality name → the icon kind (`pdf` / `document` read as a file). */
export function modalityOf(raw: string): Modality | undefined {
  const k = raw.trim().toLowerCase();
  if (k === 'text') return 'text';
  if (k === 'image' || k === 'images' || k === 'vision') return 'image';
  if (k === 'audio' || k === 'speech' || k === 'voice') return 'audio';
  if (k === 'video') return 'video';
  if (k === 'file' || k === 'pdf' || k === 'document' || k === 'files') return 'file';
  return undefined;
}

/** The known modalities of a list, deduplicated, in a fixed order. */
export function modalities(list: readonly string[] | undefined): Modality[] {
  const set = new Set((list ?? []).map(modalityOf).filter((m): m is Modality => !!m));
  return ORDER.filter((m) => set.has(m));
}

/** The model's input icons and — only when it makes something besides text — its output icons. */
export function rowModalities(m: Pick<ModelInfo, 'input' | 'output'>): { input: Modality[]; output: Modality[] } {
  const output = modalities(m.output);
  return { input: modalities(m.input), output: output.some((o) => o !== 'text') ? output : [] };
}

const ACCEPTS: Record<Modality, string> = {
  text: 'Accepts text',
  image: 'Accepts images',
  audio: 'Accepts audio',
  video: 'Accepts video',
  file: 'Accepts files (PDF)',
};
const MAKES: Record<Modality, string> = {
  text: 'Generates text',
  image: 'Generates images',
  audio: 'Generates audio',
  video: 'Generates video',
  file: 'Generates files',
};

const icon = (m: Modality): JSX.Element => {
  const p = { size: 13, 'stroke-width': 2, 'aria-hidden': true } as const;
  switch (m) {
    case 'text':
      return <Type {...p} />;
    case 'image':
      return <Image {...p} />;
    case 'audio':
      return <AudioLines {...p} />;
    case 'video':
      return <Video {...p} />;
    default:
      return <FileText {...p} />;
  }
};

/** The icon row (nothing when the catalogue names no modality). */
export function ModalityIcons(props: { m: Pick<ModelInfo, 'input' | 'output'> }) {
  const mods = () => rowModalities(props.m);
  return (
    <Show when={mods().input.length > 0 || mods().output.length > 0}>
      <span class="mp-mods" data-testid="model-modalities">
        <For each={mods().input}>
          {(m) => (
            <span class="mp-mod" role="img" aria-label={ACCEPTS[m]} title={ACCEPTS[m]} data-mod={m}>
              {icon(m)}
            </span>
          )}
        </For>
        <Show when={mods().output.length > 0}>
          <span class="mp-mod-arrow" aria-hidden="true">
            <ArrowRight size={11} stroke-width={2} />
          </span>
          <For each={mods().output}>
            {(m) => (
              <span class="mp-mod mp-mod-out" role="img" aria-label={MAKES[m]} title={MAKES[m]} data-mod-out={m}>
                {icon(m)}
              </span>
            )}
          </For>
        </Show>
      </span>
    </Show>
  );
}
