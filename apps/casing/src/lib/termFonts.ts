/**
 * Terminal fonts (UNI-127): JetBrains Mono Variable first, then the
 * bundled Symbols Nerd Font Mono (style.css @font-face) for powerline /
 * devicons / Material Design glyphs, so TUIs like the unipi footer,
 * herdr and starship prompts render icons instead of tofu.
 *
 * xterm measures its cell from the font at open time, so the terminal
 * must only open once both faces are loaded (bounded by a timeout), and
 * re-measure if a face finishes later.
 */
export const TERM_FONT_FAMILY = "'JetBrains Mono Variable', 'Symbols Nerd Font Mono', monospace";

/** A Nerd Font codepoint (powerline right arrow) to force the symbols face to load. */
const NERD_PROBE = '\ue0b0';

type FontFaceSetLike = {
  load: (font: string, text?: string) => Promise<unknown>;
  addEventListener?: (type: string, listener: () => void) => void;
  removeEventListener?: (type: string, listener: () => void) => void;
};

function fontSet(): FontFaceSetLike | undefined {
  return typeof document !== 'undefined'
    ? (document as Document & { fonts?: FontFaceSetLike }).fonts
    : undefined;
}

/** Resolves once both terminal faces are loaded, or after `timeoutMs`. */
export async function termFontsReady(fontSize = 14, timeoutMs = 2000): Promise<boolean> {
  const fonts = fontSet();
  if (!fonts?.load) return true;
  const load = Promise.all([
    fonts.load(`${fontSize}px 'JetBrains Mono Variable'`, 'Mm'),
    fonts.load(`${fontSize}px 'Symbols Nerd Font Mono'`, NERD_PROBE),
  ]).then(
    () => true,
    () => false,
  );
  const timeout = new Promise<boolean>((resolve) => setTimeout(() => resolve(false), timeoutMs));
  return Promise.race([load, timeout]);
}

/** Calls `onLoaded` whenever a font face finishes loading later on. */
export function onLateFontLoad(onLoaded: () => void): () => void {
  const fonts = fontSet();
  if (!fonts?.addEventListener) return () => {};
  fonts.addEventListener('loadingdone', onLoaded);
  return () => fonts.removeEventListener?.('loadingdone', onLoaded);
}
