/**
 * Web build stub (vite `--mode web`, see vite.config.ts `webStubs`): the
 * app-only screens and layers (terminal, chat, files, editor, previews)
 * need the native core, which a browser doesn't have — `<Requires>` shows
 * the "Not available in the web version" notice instead. Swapping their
 * modules for this keeps xterm, CodeMirror, pdf.js and the chat stack out
 * of the bundle the kanboard daemon embeds. Phone/desktop builds never
 * see this file.
 */
export default function AppOnly() {
  return null;
}
export const ChatLayer = AppOnly;
export const ReviewViewer = AppOnly;
export const Previewer = AppOnly;
