/**
 * pi built-in routing (WP-A25, G-15): every built-in pi's `/` menu offers has
 * an **app surface** — the "only the terminal can show this" toast is gone
 * (zcode-audit G-15, spec app.md §5 WP-A25).
 *
 *   /export         → the shown chat's Export (Z-20: `_oino/export_jsonl`)
 *   /import         → the "Import session…" dialog (`_oino/import_jsonl`)
 *   /clone          → the chat ⋯ "Duplicate" action (`session_fork`, no entry)
 *   /share          → dropped (no share service): a notice pointing at Export
 *   /scoped-models  → the model picker, "Cycle set" open (`enabledModels`)
 *   /login /logout  → Settings › Providers (`_oino/auth_providers`, `_oino/login`)
 *   /settings       → Settings
 *   /changelog      → About › What's new
 *   /hotkeys        → the Ctrl+/ keyboard list
 *   /trust          → dropped: core has no trust list to manage (Z: same notice)
 *   /bug            → About › Report a bug (the repository's issue form)
 *   /quit           → Quit (the desktop app; a notice elsewhere)
 *
 * The table is **data** so two things can walk it: the reachability test
 * (`lib/reachability.ts`, app.md §5 A25 acceptance) and the command center,
 * which registers one action per row. `builtinPlan` is pure; `runBuiltin`
 * applies a plan through injected effects so it is testable without a shell.
 */
import type { ChatActionId } from '../../components/shell/ConversationHeader';

/** The issue form of the project (§dist: GitHub Releases). */
export const BUG_URL = 'https://github.com/Neuron-Mr-White/Oino/issues/new';

/** The app surface a built-in opens. These ids are the reachability table's vocabulary. */
export type BuiltinSurface =
  | 'chat:export'
  | 'chat:duplicate'
  | 'app:import-session'
  | 'app:notice-share-dropped'
  | 'app:notice-trust-dropped'
  | 'app:model-picker-cycle-set'
  | 'settings:providers'
  | 'route:/settings'
  | 'settings:about'
  | 'app:keyboard-shortcuts'
  | 'app:quit';

/** What a built-in does once routed (pure data; `runBuiltin` executes it). */
export type BuiltinAction =
  | { do: 'chat-action'; id: ChatActionId }
  | { do: 'import' }
  | { do: 'notice'; text: string }
  | { do: 'picker-cycle-set' }
  | { do: 'settings'; section: string; anchor?: string }
  | { do: 'hotkeys' }
  /** About scrolled to the bug row, whose action is the repository's issue form. */
  | { do: 'bug' }
  | { do: 'quit' }
  /** Types a command into the shown chat (an extension's own panel opener). */
  | { do: 'command'; command: string };

export type BuiltinPlan = { surface: BuiltinSurface; action: BuiltinAction; title: string; keywords?: string };

/**
 * One row per pi built-in (the G-15 list, in pi's order). `title` is the
 * command-center entry (the same action from the palette, app.md §5 A25).
 */
export const BUILTIN_ROUTES: Record<string, BuiltinPlan> = {
  export: {
    surface: 'chat:export',
    action: { do: 'chat-action', id: 'export' },
    title: 'Export this chat (JSONL)',
    keywords: 'export download transcript jsonl save',
  },
  import: {
    surface: 'app:import-session',
    action: { do: 'import' },
    title: 'Import a session from JSONL…',
    keywords: 'import resume jsonl file',
  },
  clone: {
    surface: 'chat:duplicate',
    action: { do: 'chat-action', id: 'duplicate' },
    title: 'Duplicate this chat',
    keywords: 'clone duplicate copy session',
  },
  share: {
    surface: 'app:notice-share-dropped',
    action: { do: 'notice', text: 'Sharing as a secret gist is not built — Export gives you the session file instead.' },
    title: 'Share this chat',
    keywords: 'share gist link',
  },
  'scoped-models': {
    surface: 'app:model-picker-cycle-set',
    action: { do: 'picker-cycle-set' },
    title: 'Choose the models to cycle',
    keywords: 'scoped models cycle enable disable set',
  },
  login: {
    surface: 'settings:providers',
    action: { do: 'settings', section: 'providers', anchor: 'sec-providers' },
    title: 'Sign in to a provider',
    keywords: 'login sign in oauth api key',
  },
  logout: {
    surface: 'settings:providers',
    action: { do: 'settings', section: 'providers', anchor: 'sec-providers' },
    title: 'Sign out of a provider',
    keywords: 'logout sign out credentials',
  },
  settings: {
    surface: 'route:/settings',
    action: { do: 'settings', section: 'general' },
    title: 'Open Settings',
    keywords: 'settings preferences',
  },
  changelog: {
    surface: 'settings:about',
    action: { do: 'settings', section: 'about', anchor: 'about-whats-new' },
    title: "What's new",
    keywords: 'changelog release notes version history',
  },
  hotkeys: {
    surface: 'app:keyboard-shortcuts',
    action: { do: 'hotkeys' },
    title: 'Keyboard shortcuts',
    keywords: 'hotkeys keys keyboard shortcuts',
  },
  trust: {
    surface: 'app:notice-trust-dropped',
    action: {
      do: 'notice',
      text: 'The host decides project trust when it opens the folder; there is no trust list to edit yet.',
    },
    title: 'Save the project trust decision',
    keywords: 'trust project folder',
  },
  bug: {
    surface: 'settings:about',
    action: { do: 'bug' },
    title: 'Report a bug',
    keywords: 'bug issue report problem',
  },
  quit: {
    surface: 'app:quit',
    action: { do: 'quit' },
    title: 'Quit Unipi',
    keywords: 'quit exit close app',
  },
};

/** The routing plan for `/name` (no leading slash), or undefined when pi has no such built-in. */
export function builtinPlan(name: string): BuiltinPlan | undefined {
  return BUILTIN_ROUTES[name];
}

/** Every built-in name the router knows, in the table's order. */
export function routedBuiltinNames(): string[] {
  return Object.keys(BUILTIN_ROUTES);
}

/** The effects a built-in needs. The composer wires the real ones; tests pass spies. */
export type BuiltinDeps = {
  /** The chat the command acts on (the shown session, else the most recent one). */
  chat?: { hostId: string; sessionId: string };
  /** Runs a registered chat ⋯ action for that chat; false = nothing is plugged in. */
  openChatAction: (id: ChatActionId) => boolean;
  /** Opens the "Import session…" dialog (needs a chat too). */
  openImport: () => boolean;
  /** The model picker with the "Cycle set" section open. */
  openPickerCycleSet: () => boolean;
  /** Settings, optionally scrolled to an anchor. */
  openSettings: (section: string, anchor?: string) => void;
  /** The Ctrl+/ list. */
  openHotkeys: () => void;
  /** About, scrolled to the "Report a bug" row, then the issue form in the browser. */
  reportBug: (url: string) => void;
  notice: (text: string) => void;
  quit: () => void;
  /** Types `/command` into the shown chat (the extension panel openers). */
  runInChat: (command: string) => boolean;
};

/** No chat is shown: the R8 wording for every chat-bound built-in. */
export const NO_CHAT_NOTICE = 'Open a chat first';

/**
 * Applies one built-in's plan. Returns false when the built-in is unknown or
 * needs a chat that is not there (the caller then offers "New chat here", R8).
 */
export function runBuiltin(name: string, deps: BuiltinDeps): boolean {
  const plan = builtinPlan(name);
  if (!plan) return false;
  const action = plan.action;
  switch (action.do) {
    case 'chat-action':
      return deps.openChatAction(action.id);
    case 'import':
      return deps.openImport();
    case 'notice':
      deps.notice(action.text);
      return true;
    case 'picker-cycle-set':
      return deps.openPickerCycleSet();
    case 'settings':
      deps.openSettings(action.section, action.anchor);
      return true;
    case 'hotkeys':
      deps.openHotkeys();
      return true;
    case 'bug':
      deps.reportBug(BUG_URL);
      return true;
    case 'quit':
      deps.quit();
      return true;
    case 'command':
      return deps.runInChat(action.command);
    default:
      return false;
  }
}

/**
 * `/clone`'s row in the chat ⋯ menu: a fork at the leaf (`session_fork` with no
 * entry id, host.md §4.2) is pi's `clone`. Kept here so the menu row and the
 * built-in share one label.
 */
export const DUPLICATE_LABEL = 'Duplicate';

/* ── P0-02: the built-ins that act on the chat ─────────────────────────── */

/**
 * The engine's chat built-ins (oino's `get_commands` does not list them):
 * each maps to an oino RPC or an app action, run through injected effects.
 *
 *   /model [name]      → the model picker, or `set_model` for a named model
 *   /thinking [level]  → the thinking menu, or `set_thinking_level`
 *   /compact [focus]   → `compact{customInstructions}`
 *   /new               → a new chat in this folder (the draft route)
 *   /resume            → the command center on Sessions
 *   /fork              → the ⋯ Fork action (`session_fork`)
 *   /tree              → the Tree tab
 *   /name [title]      → the Rename dialog, or `session_rename`
 *   /copy              → `get_last_assistant_text` → clipboard
 *   /session           → the Session modal's Info tab
 *
 * `chat: false` rows also work on the new-chat draft (before a session exists).
 */
export const SESSION_BUILTINS: Record<string, { chat: boolean }> = {
  model: { chat: false },
  thinking: { chat: false },
  compact: { chat: true },
  new: { chat: false },
  resume: { chat: false },
  fork: { chat: true },
  tree: { chat: true },
  name: { chat: true },
  copy: { chat: true },
  session: { chat: true },
};

/** The effects the chat built-ins need (the composer wires the real ones). */
export type SessionBuiltinDeps = {
  model: (query?: string) => void;
  thinking: (level?: string) => void;
  compact: (focus?: string) => void;
  newChat: () => void;
  resume: () => void;
  fork: () => void;
  tree: () => void;
  rename: (title?: string) => void;
  copyLast: () => void;
  sessionInfo: () => void;
};

/** Whether `/name` runs on the draft (no chat yet). Routed app surfaces need none either, except the chat ⋯ ones. */
export function builtinWorksWithoutChat(name: string): boolean {
  const own = SESSION_BUILTINS[name];
  if (own) return !own.chat;
  const plan = BUILTIN_ROUTES[name];
  return !!plan && plan.action.do !== 'chat-action' && plan.action.do !== 'import';
}

/** Runs a chat built-in; false when `name` is not one. */
export function runSessionBuiltin(name: string, arg: string | undefined, deps: SessionBuiltinDeps): boolean {
  const a = arg?.trim() || undefined;
  switch (name) {
    case 'model':
      deps.model(a);
      return true;
    case 'thinking':
      deps.thinking(a);
      return true;
    case 'compact':
      deps.compact(a);
      return true;
    case 'new':
      deps.newChat();
      return true;
    case 'resume':
      deps.resume();
      return true;
    case 'fork':
      deps.fork();
      return true;
    case 'tree':
      deps.tree();
      return true;
    case 'name':
      deps.rename(a);
      return true;
    case 'copy':
      deps.copyLast();
      return true;
    case 'session':
      deps.sessionInfo();
      return true;
    default:
      return false;
  }
}
