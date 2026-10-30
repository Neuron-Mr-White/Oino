import { Show, type JSX } from "solid-js";
import type { PathKind } from "../lib/schemas";
import { hostClient } from "../lib/host/factory";
import type { HostConn } from "../lib/host/client";
import { NoHostState, OfflineState } from "./EmptyState";
import { setAddHostOpen } from "./Pairing";
export type OnlineState = "connected" | "connecting" | "error" | "unknown";
export const pathLabels: Record<PathKind, string> = {
  lan: "LAN",
  tailnet: "Tailnet",
  direct: "Direct",
  relay: "Relay",
  local: "Local",
  unknown: "Unknown",
};
export function PathChip(props: { path: PathKind; dense?: boolean }) {
  const dense = () => props.dense ?? false;
  return (
    <span
      class={
        props.path === "relay"
          ? "text-amber"
          : props.path === "unknown"
            ? "text-text-dim"
            : "text-wm-cyan"
      }
      style={{
        background: dense()
          ? props.path === "relay"
            ? "#3a2f14"
            : props.path === "unknown"
              ? "#292521"
              : "#0d2b36"
          : props.path === "relay"
            ? "rgb(240 168 24 / 14%)"
            : props.path === "unknown"
              ? "rgb(110 101 93 / 14%)"
              : "rgb(40 190 240 / 14%)",
        height: dense() ? "20px" : "24px",
        padding: dense() ? "0 8px" : "0 10px",
        "border-radius": "999px",
        display: "inline-flex",
        "align-items": "center",
        gap: dense() ? "4px" : "6px",
        "font-size": dense() ? "11px" : "12px",
        "font-weight": 500,
      }}
    >
      <span
        style={{
          width: dense() ? "5px" : "6px",
          height: dense() ? "5px" : "6px",
          "border-radius": "50%",
          background: "currentColor",
        }}
      />
      {pathLabels[props.path]}
    </span>
  );
}
export function OnlineDot(props: { state: OnlineState }) {
  return (
    <span
      aria-label={props.state}
      style={{"font-size":"6px", "line-height":"6px"}}
      class={
        {
          connected: "text-wm-green",
          connecting: "text-amber",
          error: "text-wm-red",
          unknown: "text-text-dim",
        }[props.state]
      }
    >
      ●
    </span>
  );
}
export function SessionSkeleton() {
  return (
    <div role="status" aria-label="Loading sessions" class="space-y-3">
      <div class="skeleton h-24 rounded-card bg-surface-2" />
      <div class="skeleton h-24 rounded-card bg-surface-2" />
    </div>
  );
}

/** A host's connection state from its HostClient (Z-90); `undefined` = no host. */
export function hostConnState(hostId: string | undefined): HostConn | undefined {
  return hostId ? hostClient(hostId).conn() : undefined;
}

/**
 * The one gate for host-dependent content (Z-90, WP-A1): no host → the shared
 * NoHostState ("Add a PC"); host offline (its HostClient says so, or the
 * caller saw an unreachable error) → the shared OfflineState with Retry
 * (re-checks the connection, then `onRetry`); otherwise the children.
 */
export function HostGate(props: {
  hostId: string | undefined;
  pcName?: string;
  /** "its sessions", "its files"… (NoHostState wording). */
  what: string;
  compact?: boolean;
  /** Extra offline signal from the caller (e.g. its first request failed). */
  offline?: boolean;
  message?: string;
  hint?: string;
  onRetry?: () => void;
  onAdd?: () => void;
  children: JSX.Element;
}) {
  const conn = () => hostConnState(props.hostId);
  const offline = () => conn()?.state === "offline" || !!props.offline;
  const retry = () => {
    const id = props.hostId;
    if (!id) return;
    void hostClient(id).retry().finally(() => props.onRetry?.());
  };
  return (
    <Show when={props.hostId} fallback={<NoHostState what={props.what} onAdd={props.onAdd ?? (() => setAddHostOpen(true))} />}>
      <Show
        when={!offline()}
        fallback={
          <OfflineState
            compact={props.compact}
            pc={props.pcName}
            message={conn()?.message ?? props.message}
            hint={conn()?.hint ?? props.hint ?? "Is unipi-host running on the PC?"}
            onRetry={retry}
          />
        }
      >
        {props.children}
      </Show>
    </Show>
  );
}
