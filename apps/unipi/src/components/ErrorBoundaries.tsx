/**
 * Crash containment (WP-A9, Z-89): one Solid `ErrorBoundary` per region, so a
 * throwing view never takes the shell down.
 *
 * - `RootBoundary` wraps the whole app: Retry (re-render) / Reload app.
 * - `ConversationBoundary` wraps the timeline + dock: the sidebar, panes and
 *   every other conversation stay alive; its Retry remounts the region.
 * - `PaneTabBoundary` wraps one side-pane / bottom-panel tab body.
 * - `SlotBoundary` wraps one ui-protocol slot entry.
 *
 * The chat itself is a sibling region of every pane tab, so a crashing tab
 * never stops the stream (it keeps applying events under its boundary).
 */
import { ErrorBoundary, type JSX } from 'solid-js';
import { RefreshCw, RotateCcw, TriangleAlert } from 'lucide-solid';
import { EmptyState } from './EmptyState';

const messageOf = (error: unknown): string => (error instanceof Error ? error.message || error.name : String(error));

/** The one crash card: what broke, and the actions that can bring it back. */
export function CrashCard(props: { title: string; error: unknown; actions: JSX.Element; testId: string; compact?: boolean }) {
  return (
    <EmptyState
      compact={props.compact}
      tone="error"
      role="alert"
      icon={<TriangleAlert size={props.compact ? 20 : 24} />}
      title={props.title}
      body={messageOf(props.error)}
      testId={props.testId}
      action={props.actions}
    />
  );
}

/** Reload the whole app (the root boundary's last resort). */
export function reloadApp(): void {
  try {
    location.reload();
  } catch {
    // a test/jsdom env has no navigation: Retry stays
  }
}

/** The app itself failed to render (Z-89): Retry / Reload app. */
export function RootBoundary(props: { children: JSX.Element; onReload?: () => void }) {
  return (
    <ErrorBoundary
      fallback={(error, reset) => (
        <div class="crash-root" data-testid="root-crashed">
          <CrashCard
            title="UniPi hit a problem"
            error={error}
            testId="crash-card"
            actions={
              <>
                <button type="button" class="btn btn-secondary" onClick={() => reset()}>
                  <RefreshCw size={16} /> Retry
                </button>
                <button type="button" class="btn btn-ghost" onClick={() => (props.onReload ?? reloadApp)()}>
                  <RotateCcw size={16} /> Reload app
                </button>
              </>
            }
          />
        </div>
      )}
    >
      {props.children}
    </ErrorBoundary>
  );
}

/** One conversation (timeline + dock) failed to render; the rest of the app keeps working. */
export function ConversationBoundary(props: { children: JSX.Element }) {
  return (
    <ErrorBoundary
      fallback={(error, reset) => (
        <div class="crash-conv" data-testid="conv-crashed">
          <CrashCard
            title="This conversation stopped working"
            error={error}
            testId="crash-card"
            actions={
              <>
                <button type="button" class="btn btn-secondary" onClick={() => reset()}>
                  <RefreshCw size={16} /> Retry
                </button>
                <button type="button" class="btn btn-ghost" onClick={() => reloadApp()}>
                  <RotateCcw size={16} /> Reload app
                </button>
              </>
            }
          />
        </div>
      )}
    >
      {props.children}
    </ErrorBoundary>
  );
}

/** One pane tab body failed to render (§3.8: every tab sits in its own boundary). */
export function PaneTabBoundary(props: { title: string; children: JSX.Element }) {
  return (
    <ErrorBoundary
      fallback={(error, reset) => (
        <CrashCard
          compact
          title={`${props.title} stopped working`}
          error={error}
          testId="tab-crashed"
          actions={
            <button type="button" class="btn btn-secondary btn-sm" onClick={() => reset()}>
              Reload tab
            </button>
          }
        />
      )}
    >
      {props.children}
    </ErrorBoundary>
  );
}

/** One ui-protocol slot entry failed to render: Retry, or report it to the extension. */
export function SlotBoundary(props: { entryId: string; onReport?: (entryId: string, error: unknown) => void; children: JSX.Element }) {
  return (
    <ErrorBoundary
      fallback={(error, reset) => (
        <CrashCard
          compact
          title="This panel failed to render"
          error={error}
          testId="ui-entry-failed"
          actions={
            <>
              <button type="button" class="btn btn-secondary btn-sm" onClick={() => reset()}>
                Retry
              </button>
              <button type="button" class="btn btn-ghost btn-sm" onClick={() => props.onReport?.(props.entryId, error)}>
                Report
              </button>
            </>
          }
        />
      )}
    >
      {props.children}
    </ErrorBoundary>
  );
}
