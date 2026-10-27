//! The session budget and the write gate (unipi `guard.ts`).
//!
//! Bash calls into `oino-kanboard` are split into always-allowed reads and writes. A
//! `/kanboard-do` grants task SLOTS (each `start` uses one) and a WRITE budget (add,
//! edit, link, order, move backlog↔todo, note on tasks you don't hold). Always free: reads,
//! and `finish`, `move <ID> blocked`, `note <ID>` and `attach <ID>` on claims this session
//! started — a started task can always be closed. Autowork lifts both budgets (the runaway
//! guard is the per-turn add cap). In child sessions every write is refused: the lead updates
//! the board.
//!
//! unipi probed claim ownership lazily through an async callback; here the caller first asks
//! [`WriteGuard::claim_probes`] for the ids whose ownership can matter, probes them, and passes
//! the answers to [`WriteGuard::check`]. A failed probe counts as "not owned" (never a free
//! write), exactly like unipi.

use std::collections::HashMap;

use crate::shell::{Invocation, invocations};

/// Refusal for every write from a child session.
pub const CHILD_WRITE_REFUSAL: &str =
    "board writes are the lead's job — report this to the lead; the lead updates the board";
/// Refusal for the subcommands removed with the runner.
pub const REMOVED_REFUSAL: &str =
    "removed: the session works tasks itself (no runner/queue/strategy)";
/// Refusal when the task slots are spent.
pub const SLOTS_USED_UP: &str = "no task slots left — tell the user you can't start more now: raise kanboard.doTasks or run /kanboard-do again for the next batch";
/// Refusal when the write budget is spent.
pub const WRITES_USED_UP: &str = "kanboard write budget used up — run /kanboard-do to reload (reads, finish, and blocked/note on your own claims are always free)";
/// Default `kanboard.doTasks`.
pub const DEFAULT_DO_TASKS: u64 = 5;
/// Default `kanboard.doWrites`.
pub const DEFAULT_DO_WRITES: u64 = 10;
/// Default `kanboard.turnAddLimit`.
pub const ADD_CAP: u64 = 20;

/// Subcommands removed with the runner: refused outright.
const REMOVED: &[&str] = &["queue", "unqueue", "claim-next", "set-run"];

/// Refusal past the add cap.
#[must_use]
pub fn add_cap_reason(limit: u64) -> String {
    format!("at most {limit} new tasks per turn")
}

/// Budget left this session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Budget {
    /// Task slots (`start`).
    pub slots: u64,
    /// Board writes.
    pub writes: u64,
    /// Autowork on (both budgets lifted).
    pub autowork: bool,
}

/// The write guard of one session. Budgets persist across turns until spent;
/// `/kanboard-do` tops up without stacking past N.
#[derive(Clone, Debug, Default)]
pub struct WriteGuard {
    slots: u64,
    writes: u64,
    autowork: bool,
    do_open: bool,
    sent_at: i64,
    adds: u64,
}

impl WriteGuard {
    /// `-do`: slots = max(slots, `do_tasks`), writes = max(writes, `do_writes`); resets the
    /// add cap.
    pub fn open(&mut self, do_tasks: u64, do_writes: u64) {
        self.slots = self.slots.max(do_tasks);
        self.writes = self.writes.max(do_writes);
        self.adds = 0;
        self.do_open = true;
    }

    /// Autowork on/off: board writes (and starts) become free.
    pub fn set_autowork(&mut self, on: bool) {
        self.autowork = on;
    }

    /// `-do off`: slots = writes = 0.
    pub fn revoke(&mut self) {
        self.slots = 0;
        self.writes = 0;
        self.do_open = false;
    }

    /// Budget left this session.
    #[must_use]
    pub fn remaining(&self) -> Budget {
        Budget {
            slots: self.slots,
            writes: self.writes,
            autowork: self.autowork,
        }
    }

    /// Arms the `agent_end` closer right after the `-do` prompt was sent.
    pub fn note_sent(&mut self, now_ms: i64) {
        self.sent_at = now_ms;
    }

    /// The budget persists across turns; this only closes the `-do` window label. A late end
    /// from the previous turn (< 150 ms after the send) is ignored.
    pub fn on_agent_end(&mut self, now_ms: i64) -> bool {
        if !self.do_open || now_ms - self.sent_at < 150 {
            return false;
        }
        self.do_open = false;
        true
    }

    /// Task ids whose claim ownership can make a write free (`move <ID> blocked`, `note <ID>`,
    /// `attach <ID>`). Probing an id that turns out not to matter is harmless (reads only).
    #[must_use]
    pub fn claim_probes(command: &str) -> Vec<String> {
        let mut ids: Vec<String> = Vec::new();
        for inv in invocations(command) {
            if !inv.is_known() || inv.is_readonly() {
                continue;
            }
            let probe = inv.is_move_blocked() || inv.sub == "note" || inv.sub == "attach";
            if let (true, Some(id)) = (probe, inv.first_positional())
                && !ids.iter().any(|known| known == id)
            {
                ids.push(id.to_owned());
            }
        }
        ids
    }

    fn count_add(&mut self, inv: &Invocation, limit: u64) -> Option<String> {
        if inv.sub != "add" {
            return None;
        }
        self.adds += 1;
        (limit > 0 && self.adds > limit).then(|| add_cap_reason(limit))
    }

    /// `None` when `command` is allowed; otherwise the block reason. Charges the budget for
    /// every allowed write, in order (a block mid-way keeps the earlier charges, like unipi).
    /// `owned` maps probed task ids to "claimed by this session".
    pub fn check(
        &mut self,
        command: &str,
        is_child: bool,
        add_limit: u64,
        owned: &HashMap<String, bool>,
    ) -> Option<String> {
        let owns = |id: &str| owned.get(id).copied().unwrap_or(false);
        for inv in invocations(command) {
            // An unknown subcommand (a typo, a bare binary) is the binary's own usage error —
            // never a reason to block or charge the whole call.
            if !inv.is_known() || inv.is_readonly() {
                continue;
            }
            if is_child {
                return Some(CHILD_WRITE_REFUSAL.to_owned());
            }
            if REMOVED.contains(&inv.sub.as_str()) || inv.is_removed_edit() {
                return Some(REMOVED_REFUSAL.to_owned());
            }
            // The add cap applies always (runaway guard), before any charging.
            if let Some(reason) = self.count_add(&inv, add_limit) {
                return Some(reason);
            }
            if inv.sub == "finish" {
                continue;
            }
            if inv.is_move_blocked() && inv.first_positional().is_some_and(owns) {
                continue;
            }
            if (inv.sub == "note" || inv.sub == "attach")
                && inv.first_positional().is_some_and(owns)
            {
                continue;
            }
            if self.autowork {
                continue;
            }
            if inv.sub == "start" {
                if self.slots == 0 {
                    return Some(SLOTS_USED_UP.to_owned());
                }
                self.slots -= 1;
                continue;
            }
            if self.writes == 0 {
                return Some(WRITES_USED_UP.to_owned());
            }
            self.writes -= 1;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: &str = "oino-kanboard --actor agent --project p";

    fn none() -> HashMap<String, bool> {
        HashMap::new()
    }

    fn owned(ids: &[&str]) -> HashMap<String, bool> {
        ids.iter().map(|id| ((*id).to_owned(), true)).collect()
    }

    fn check(g: &mut WriteGuard, cmd: &str) -> Option<String> {
        g.check(cmd, false, ADD_CAP, &none())
    }

    #[test]
    fn reads_are_always_free() {
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
    fn children_refuse_every_write() {
        let mut g = WriteGuard::default();
        g.open(5, 10);
        g.set_autowork(true);
        for write in [
            "finish A-1 --comment x",
            "move A-1 blocked --comment x",
            "note A-1 x",
            "add t",
        ] {
            assert_eq!(
                g.check(&format!("{B} {write}"), true, ADD_CAP, &owned(&["A-1"]))
                    .as_deref(),
                Some(CHILD_WRITE_REFUSAL),
                "{write}"
            );
        }
        assert_eq!(
            g.check(&format!("{B} show A-1"), true, ADD_CAP, &none()),
            None
        );
    }

    #[test]
    fn removed_subcommands_are_refused_before_any_charge() {
        let mut g = WriteGuard::default();
        g.open(5, 10);
        g.set_autowork(true);
        for cmd in [
            "queue A-1",
            "unqueue A-1",
            "claim-next",
            "set-run A-1",
            "edit A-1 --strategy swarm",
            "edit A-1 --plan=true",
        ] {
            assert_eq!(
                check(&mut g, &format!("{B} {cmd}")).as_deref(),
                Some(REMOVED_REFUSAL),
                "{cmd}"
            );
        }
        assert_eq!(g.remaining().writes, 10);
    }

    #[test]
    fn unknown_subcommands_are_skipped() {
        let mut g = WriteGuard::default();
        assert_eq!(check(&mut g, &format!("{B} done A-1")), None);
        assert_eq!(check(&mut g, B), None);
        assert_eq!(check(&mut g, &format!("{B} done A-1 && {B} list")), None);
    }

    #[test]
    fn start_costs_a_slot_writes_cost_the_budget() {
        let mut g = WriteGuard::default();
        assert_eq!(
            check(&mut g, &format!("{B} start A-1")).as_deref(),
            Some(SLOTS_USED_UP)
        );
        assert_eq!(
            check(&mut g, &format!("{B} add t")).as_deref(),
            Some(WRITES_USED_UP)
        );
        g.open(2, 1);
        assert_eq!(check(&mut g, &format!("{B} start A-1")), None);
        assert_eq!(check(&mut g, &format!("{B} start A-2")), None);
        assert_eq!(
            check(&mut g, &format!("{B} start A-3")).as_deref(),
            Some(SLOTS_USED_UP)
        );
        assert_eq!(check(&mut g, &format!("{B} add t")), None);
        assert_eq!(
            check(&mut g, &format!("{B} link A-2 --after A-1")).as_deref(),
            Some(WRITES_USED_UP)
        );
    }

    #[test]
    fn finish_is_always_free() {
        let mut g = WriteGuard::default();
        assert_eq!(
            check(&mut g, &format!("{B} finish A-1 --comment done")),
            None
        );
        assert_eq!(g.remaining(), Budget::default());
    }

    #[test]
    fn move_todo_costs_a_write_even_on_an_owned_claim() {
        let mut g = WriteGuard::default();
        let reason = g.check(
            &format!("{B} move A-1 todo"),
            false,
            ADD_CAP,
            &owned(&["A-1"]),
        );
        assert_eq!(reason.as_deref(), Some(WRITES_USED_UP));
    }

    #[test]
    fn own_claim_closes_and_notes_are_free() {
        let mut g = WriteGuard::default();
        let own = owned(&["A-1"]);
        assert_eq!(
            g.check(
                &format!("{B} move A-1 blocked --comment x"),
                false,
                ADD_CAP,
                &own
            ),
            None
        );
        assert_eq!(
            g.check(&format!("{B} note A-1 hi"), false, ADD_CAP, &own),
            None
        );
        assert_eq!(
            g.check(&format!("{B} attach A-1 /tmp/f"), false, ADD_CAP, &own),
            None
        );
        // Foreign claims (or a failed probe = absent) cost a write.
        assert_eq!(
            g.check(
                &format!("{B} move A-2 blocked --comment x"),
                false,
                ADD_CAP,
                &own
            )
            .as_deref(),
            Some(WRITES_USED_UP)
        );
        assert_eq!(
            g.check(&format!("{B} note A-2 hi"), false, ADD_CAP, &own)
                .as_deref(),
            Some(WRITES_USED_UP)
        );
    }

    #[test]
    fn claim_probes_list_only_the_ids_that_can_be_free() {
        let cmd = format!(
            "{B} move A-1 blocked --comment x && {B} note A-2 y && {B} attach A-2 f && {B} start A-3 && {B} show A-4"
        );
        assert_eq!(WriteGuard::claim_probes(&cmd), ["A-1", "A-2"]);
    }

    #[test]
    fn autowork_lifts_budgets_but_not_the_add_cap() {
        let mut g = WriteGuard::default();
        g.set_autowork(true);
        assert_eq!(check(&mut g, &format!("{B} start A-1")), None);
        assert_eq!(check(&mut g, &format!("{B} link A-2 --after A-1")), None);
        for _ in 0..3 {
            assert_eq!(g.check(&format!("{B} add t"), false, 3, &none()), None);
        }
        assert_eq!(
            g.check(&format!("{B} add t"), false, 3, &none()),
            Some(add_cap_reason(3))
        );
    }

    #[test]
    fn revoke_zeroes_both_budgets() {
        let mut g = WriteGuard::default();
        g.open(5, 10);
        g.revoke();
        assert_eq!(
            g.remaining(),
            Budget {
                slots: 0,
                writes: 0,
                autowork: false
            }
        );
    }

    #[test]
    fn open_tops_up_without_stacking_and_resets_the_add_cap() {
        let mut g = WriteGuard::default();
        g.open(5, 10);
        assert_eq!(check(&mut g, &format!("{B} start A-1")), None);
        g.open(5, 10);
        assert_eq!(g.remaining().slots, 5);
        g.open(3, 4);
        assert_eq!(
            g.remaining(),
            Budget {
                slots: 5,
                writes: 10,
                autowork: false
            }
        );
        for _ in 0..2 {
            assert_eq!(g.check(&format!("{B} add t"), false, 2, &none()), None);
        }
        assert!(g.check(&format!("{B} add t"), false, 2, &none()).is_some());
        g.open(5, 10);
        assert_eq!(g.check(&format!("{B} add t"), false, 2, &none()), None);
    }

    #[test]
    fn agent_end_closes_the_window_label_only_after_the_echo_guard() {
        let mut g = WriteGuard::default();
        assert!(!g.on_agent_end(1_000));
        g.open(5, 10);
        g.note_sent(1_000);
        assert!(!g.on_agent_end(1_100), "late end from the previous turn");
        assert!(g.on_agent_end(1_200));
        assert!(!g.on_agent_end(1_300));
        assert_eq!(g.remaining().slots, 5, "budget persists");
    }

    #[test]
    fn add_cap_is_unlimited_at_zero_and_matches_the_default_text() {
        let mut g = WriteGuard::default();
        g.set_autowork(true);
        for _ in 0..50 {
            assert_eq!(g.check(&format!("{B} add t"), false, 0, &none()), None);
        }
        assert_eq!(add_cap_reason(ADD_CAP), "at most 20 new tasks per turn");
    }
}
