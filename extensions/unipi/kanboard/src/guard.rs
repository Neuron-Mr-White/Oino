//! The bash write gate (unipi `guard.ts`).
//!
//! Bash calls into `unipi-kanboard` are allowed for every session — ordinary chats, leads and
//! subagents alike. Nothing is charged and nothing is capped: the only refusals are the removed
//! runner edits. Reads are always free.

use crate::shell::invocations;

/// Refusal for runner remnants in an argv (`edit --strategy`/`--plan`).
pub const REMOVED_REFUSAL: &str =
    "removed: the session works tasks itself (no runner/queue/strategy)";

// Subcommands removed with the runner are no longer listed in
// `shell::KNOWN_SUBCOMMANDS` (ext-abi §5.3): like any unknown name they are the
// binary's own usage error — never blocked. `edit --strategy/--plan` still
// names the removed runner and is refused ([`REMOVED_REFUSAL`]).

/// The write guard of one session: the autowork flag the footer/status shows.
/// Nothing is charged and nothing is capped.
#[derive(Clone, Debug, Default)]
pub struct WriteGuard {
    autowork: bool,
}

impl WriteGuard {
    /// Autowork on/off (display only now that no budget lifts).
    pub fn set_autowork(&mut self, on: bool) {
        self.autowork = on;
    }

    /// Autowork is on.
    #[must_use]
    pub fn autowork(&self) -> bool {
        self.autowork
    }

    /// `None` when `command` is allowed; otherwise the block reason. Only the removed runner
    /// edits can refuse a write — every session may start, note, move, finish, link, attach
    /// and add freely, as many as it wants.
    pub fn check(&mut self, command: &str) -> Option<String> {
        for inv in invocations(command) {
            // An unknown subcommand (a typo, a bare binary) is the binary's own usage error —
            // never a reason to block the whole call.
            if !inv.is_known() || inv.is_readonly() {
                continue;
            }
            if inv.is_removed_edit() {
                return Some(REMOVED_REFUSAL.to_owned());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: &str = "unipi-kanboard --actor agent --project p";

    fn check(g: &mut WriteGuard, cmd: &str) -> Option<String> {
        g.check(cmd)
    }

    #[test]
    fn reads_are_skipped() {
        let mut g = WriteGuard::default();
        for read in [
            "list",
            "show A-1",
            "next",
            "status",
            "settings show",
            "project list",
        ] {
            assert_eq!(check(&mut g, &format!("{B} {read}")), None, "{read}");
        }
        assert_eq!(check(&mut g, "echo hi"), None);
    }

    #[test]
    fn every_session_can_write_without_a_budget() {
        let mut g = WriteGuard::default();
        // Well past the old per-session limits, in one session.
        for n in 0..50 {
            for write in [
                format!("start A-{n}"),
                format!("finish A-{n} --comment done"),
                format!("note A-{n} working"),
                format!("move A-{n} in_review"),
                format!("move A-{n} todo"),
                format!("link A-{n} --after A-0"),
                format!("attach A-{n} /tmp/f --note x"),
            ] {
                assert_eq!(check(&mut g, &format!("{B} {write}")), None, "{write}");
            }
        }
        // A child session is just a session: no child refusal.
        assert_eq!(
            check(&mut g, &format!("{B} move A-1 blocked --comment need")),
            None
        );
    }

    #[test]
    fn runner_remnants_are_usage_errors_except_removed_edits() {
        let mut g = WriteGuard::default();
        // The runner-era subcommands are pruned from KNOWN_SUBCOMMANDS: unknown
        // names are the binary's usage errors — skipped, never blocked.
        for cmd in [
            "queue A-1",
            "unqueue A-1",
            "claim-next",
            "set-run A-1",
            "serve",
            "settings",
        ] {
            assert_eq!(
                check(&mut g, &format!("{B} {cmd}")).as_deref(),
                None,
                "{cmd}"
            );
        }
        // `edit --strategy/--plan` still names the removed runner.
        for cmd in ["edit A-1 --strategy swarm", "edit A-1 --plan=true"] {
            assert_eq!(
                check(&mut g, &format!("{B} {cmd}")).as_deref(),
                Some(REMOVED_REFUSAL),
                "{cmd}"
            );
        }
    }

    #[test]
    fn unknown_subcommands_are_skipped() {
        let mut g = WriteGuard::default();
        assert_eq!(check(&mut g, &format!("{B} done A-1")), None);
        assert_eq!(check(&mut g, B), None);
        assert_eq!(check(&mut g, &format!("{B} done A-1 && {B} list")), None);
    }

    #[test]
    fn many_adds_in_one_call_are_allowed() {
        let mut g = WriteGuard::default();
        // No per-turn add limit: 50 adds in a single call pass.
        let many = (0..50)
            .map(|_| format!("{B} add t"))
            .collect::<Vec<_>>()
            .join(" && ");
        assert_eq!(check(&mut g, &many), None);
    }

    #[test]
    fn autowork_is_a_display_flag_only() {
        let mut g = WriteGuard::default();
        assert!(!g.autowork());
        g.set_autowork(true);
        assert!(g.autowork());
        assert_eq!(check(&mut g, &format!("{B} start A-1")), None);
    }
}
