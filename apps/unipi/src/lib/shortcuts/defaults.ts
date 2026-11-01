/**
 * The app's shortcut map (WP-A18, Z-94): ZCode's shell-level chords + the
 * unipi screen keys. Two tables:
 *
 * - `APP_BINDINGS` — rebindable, handled once by the shell (`AppShell` maps
 *   each id to its action). ZCode shell map (K/⇧P palette, P files, F find,
 *   ⌘, settings, B sidebar, J bottom panel, ⌥⌘B side pane, ⇧F full view,
 *   ⇧T reopen tab, N new chat, ⇧[ / ⇧] prev/next conversation) plus unipi's
 *   Ctrl+/ list and Ctrl+L focus composer, plus A14's ZCode set: Mod+M model
 *   menu, Mod+T thinking cycle, Mod+Shift+M working-mode cycle. P0-10:
 *   Mod+B also backgrounds a foreground subagent (the old harness's Ctrl+B)
 *   before it toggles the sidebar; Mod+Alt+P cycles the model (the TUI's
 *   Ctrl+P — Mod+P is Files here and Alt+P is the plan toggle).
 * - `SCREEN_KEYS` — display-only (Ctrl+/ list, Settings): the keys the
 *   screens handle inside themselves (unipi board/files/editor/terminal
 *   keys, kept per the Z-94 ruling).
 */

export type AppBinding = {
  id: string;
  title: string;
  group: string;
  /** Default chord(s); the first is the one Settings shows as "the" chord. */
  chords: string[];
  /** `conversation`: only when a session is shown; `app`: anywhere. */
  scope: 'app' | 'conversation';
  /** Shown but not bound here (the owning screen handles the key). */
  displayOnly?: boolean;
};

export const APP_BINDINGS: AppBinding[] = [
  { id: 'shell.palette', title: 'Command center', group: 'Shell', chords: ['Mod+K', 'Mod+Shift+P'], scope: 'app' },
  { id: 'shell.files', title: 'Command center · Files', group: 'Shell', chords: ['Mod+P'], scope: 'app' },
  { id: 'find.open', title: 'Find in chat', group: 'Shell', chords: ['Mod+F'], scope: 'conversation' },
  { id: 'shell.shortcuts', title: 'Keyboard shortcuts', group: 'Shell', chords: ['Mod+/'], scope: 'app' },
  { id: 'shell.settings', title: 'Open Settings', group: 'Shell', chords: ['Mod+,'], scope: 'app' },
  { id: 'shell.sidebar', title: 'Show / hide the sidebar (backgrounds a running subagent first)', group: 'Panels', chords: ['Mod+B'], scope: 'app' },
  { id: 'shell.side', title: 'Show / hide the side pane', group: 'Panels', chords: ['Mod+Alt+B'], scope: 'app' },
  { id: 'shell.bottom', title: 'Show / hide the bottom panel', group: 'Panels', chords: ['Mod+J'], scope: 'app' },
  { id: 'shell.fullview', title: 'Side pane full view', group: 'Panels', chords: ['Mod+Shift+F'], scope: 'app' },
  { id: 'shell.reopen', title: 'Reopen the last closed tab', group: 'Panels', chords: ['Mod+Shift+T'], scope: 'app' },
  { id: 'chat.new', title: 'New chat', group: 'Chat', chords: ['Mod+N'], scope: 'app' },
  { id: 'chat.focus', title: 'Focus the message box', group: 'Chat', chords: ['Mod+L'], scope: 'conversation' },
  { id: 'chat.prev', title: 'Previous conversation', group: 'Chat', chords: ['Mod+Shift+['], scope: 'app' },
  { id: 'chat.next', title: 'Next conversation', group: 'Chat', chords: ['Mod+Shift+]'], scope: 'app' },
  { id: 'chat.running', title: 'Running work (Session modal)', group: 'Chat', chords: ['Shift+Down'], scope: 'conversation' },
  { id: 'model.open', title: 'Change model', group: 'Chat', chords: ['Mod+M'], scope: 'conversation' },
  { id: 'model.cycle', title: 'Cycle the model (cycle set, else Fusion suggestions)', group: 'Chat', chords: ['Mod+Alt+P'], scope: 'conversation' },
  { id: 'thinking.cycle', title: 'Cycle the thinking level', group: 'Chat', chords: ['Mod+T'], scope: 'conversation' },
  { id: 'mode.cycle', title: 'Cycle the working-mode starter (Goal / Ralph / Swarm / Graph)', group: 'Chat', chords: ['Mod+Shift+M'], scope: 'conversation' },
  { id: 'chat.send', title: 'Send (while running: last Steer/Queue choice)', group: 'Chat', chords: ['Enter'], scope: 'conversation', displayOnly: true },
  { id: 'chat.newline', title: 'New line', group: 'Chat', chords: ['Shift+Enter'], scope: 'conversation', displayOnly: true },
  { id: 'chat.back', title: 'Back / close the top layer', group: 'Chat', chords: ['Esc'], scope: 'conversation', displayOnly: true },
];

export type ScreenKeyGroup = { title: string; items: Array<{ keys: string; what: string }> };

/** The unipi screen keys the Ctrl+/ list keeps (Z-94: screens own them). */
export const SCREEN_KEYS: ScreenKeyGroup[] = [
  {
    // "Chat keys": the shell chords already have a "Chat" group above.
    title: 'Chat keys',
    items: [
      { keys: 'S / Q', what: 'In the Steer/Queue choice: Steer / Queue (←/→ switch, Enter picks)' },
      { keys: 'Ctrl V', what: 'Paste an image from the clipboard as an attachment' },
      { keys: 'Drop files', what: 'Attach files dropped on the chat' },
    ],
  },
  {
    title: 'Terminal',
    items: [
      { keys: 'Ctrl Shift C / V', what: 'Copy the selection / paste' },
      { keys: 'Ctrl = / - / 0', what: 'Bigger / smaller / default font' },
      { keys: 'Shift drag', what: 'Select text while the app uses the mouse' },
    ],
  },
  {
    title: 'Files & editor',
    items: [
      { keys: '↑ ↓ Home End', what: 'Move in the file list' },
      { keys: 'Enter', what: 'Open' },
      { keys: 'Backspace', what: 'Up one folder' },
      { keys: 'F2', what: 'Rename' },
      { keys: 'Delete', what: 'Delete (asks first)' },
      { keys: 'Ctrl S / Ctrl F', what: 'Editor: save / find' },
    ],
  },
  {
    title: 'Board',
    items: [
      { keys: 'C', what: 'New task in the last lane' },
      { keys: 'J K', what: 'Move the selection down / up' },
      { keys: 'B L', what: 'Jump to the first / last card' },
      { keys: 'R', what: 'Open the card' },
      { keys: 'X', what: 'Toggle the checkbox' },
      { keys: '?', what: 'The board key list' },
    ],
  },
];
