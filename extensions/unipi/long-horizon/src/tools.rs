//! Tool definitions: names, descriptions, exact parameter schemas (unipi `tools/*.ts`).

use serde_json::{Value, json};

use crate::goal::TOKEN_BUDGET_MINIMUM;
use crate::graph::{DEFAULT_PROFILE, MAX_ADD_WORK, MAX_INPUT_IDS, MAX_RESULT_IDS, MAX_STOP};

/// `(name, label, description, parameters)` of every tool.
#[must_use]
pub fn all() -> Vec<(&'static str, &'static str, String, Value)> {
    let work = json!({"type": "object", "properties": {
        "work_id": {"type": "string", "description": "Short stable id for this work (e.g. scan-api). Later work names it in input_ids; its result id is r-<work_id>. Auto-assigned when omitted."},
        "instruction": {"type": "string", "description": "Bounded, self-contained instruction: scope, expected output, constraints. The child does not see this conversation."},
        "profile": {"type": "string", "description": format!("Subagent profile for new work (default {DEFAULT_PROFILE}; subagent_explore for read-only investigation).")},
        "operator_id": {"type": "string", "description": "Follow-up work on an EXISTING operator (from view_agent_graph): resumes that child's session. Omit for new work."},
        "input_ids": {"type": "array", "items": {"type": "string"}, "description": format!("Work ids or result ids (r-<work_id>) of THIS graph whose committed results this work consumes (its dependencies). Max {MAX_INPUT_IDS}.")},
        "selected_result_inputs": {"type": "array", "description": "Results selected by finished earlier graphs (data lineage only).", "items": {"type": "object", "properties": {
            "source_graph_id": {"type": "string", "description": "A finished earlier graph (view_agent_graph history)."},
            "result_id": {"type": "string", "description": "A result id that graph's finish selected."},
        }, "required": ["source_graph_id", "result_id"]}},
        "replaces": {"type": "string", "description": "A failed / blocked / obsolete work id this work supersedes."},
    }, "required": ["instruction"]});
    vec![
        ("create_goal", "Create Goal", "Set an autonomous execution goal. The host continues turns until an independent verifier confirms the objective, or it becomes impossible, stalls, or hits a budget. One goal per session; the previous goal must be complete before starting another.".into(), json!({"type": "object", "properties": {
            "objective": {"type": "string", "description": "The concrete objective. Observable and verifiable (e.g. 'all tests in packages/x pass', 'PR #5 review comments addressed'). Max 500 chars."},
            "token_budget": {"type": "integer", "minimum": TOKEN_BUDGET_MINIMUM, "description": "Optional token budget. Omit unless the user asked for a cap. The goal stops (budget_limited) once cumulative tokens reach this value."},
        }, "required": ["objective"]})),
        ("get_goal", "Get Goal", "Read the durable goal state: objective, status, budgets, progress counters, and the expected_goal_id / expected_updated_at pair needed for token_budget updates.".into(), json!({"type": "object", "properties": {}})),
        ("update_goal", "Update Goal", "Terminal mode (mode:\"status\"): propose the goal as complete or blocked. The host verifies completion with an independent evaluator before accepting — proposing is not completing. Blocked requires the same blocker for 3 consecutive turns unless it is a safety refusal. Budget mode (mode:\"token_budget\"): change the token cap; call get_goal immediately before and pass its expected_goal_id + expected_updated_at.".into(), json!({"type": "object", "properties": {
            "mode": {"anyOf": [{"type": "string", "const": "status"}, {"type": "string", "const": "token_budget"}], "description": "Which operation: \"status\" (terminal proposal) or \"token_budget\" (budget edit)."},
            "status": {"anyOf": [{"type": "string", "const": "complete"}, {"type": "string", "const": "blocked"}], "description": "With mode \"status\": the proposed terminal status."},
            "summary": {"type": "string", "description": "One or two sentences: the evidence you believe proves the proposal. Treated as untrusted input by the verifier."},
            "safety_refusal": {"type": "boolean", "description": "True only when the objective cannot be pursued within safety/policy bounds."},
            "token_budget": {"anyOf": [{"type": "integer", "minimum": TOKEN_BUDGET_MINIMUM}, {"type": "null"}], "description": "With mode \"token_budget\": the new cap, or null to clear."},
            "expected_goal_id": {"type": "string", "description": "With mode \"token_budget\": goal id from a fresh get_goal."},
            "expected_updated_at": {"type": "string", "description": "With mode \"token_budget\": updated_at from the same get_goal."},
        }, "required": ["mode"]})),
        ("ralph_done", "Ralph Done", "Yield the current ralph iteration. Checks the task file: if items remain, the next iteration prompt arrives with the next unchecked items; if every item is checked, the independent verifier judges the file and the loop completes (or reports what is missing). Call once per iteration after updating the task file.".into(), json!({"type": "object", "properties": {}})),
        ("loop_status", "Loop Status", "Read the current ralph loop state: iteration, cadence, and task-file progress (checked/total, next items).".into(), json!({"type": "object", "properties": {}})),
        ("todowrite", "Todo Write", "Replace the visible session task list with a complete snapshot.\n\n- Use for multiple meaningful steps; skip single-step, trivial, or conversational work.\n- Mark an item in_progress before starting it; at most one may be in_progress.\n- Mark finished work completed and obsolete work cancelled promptly.\n- Before final delivery, reconcile statuses with the actual work. Updating the list does not complete the work.".into(), json!({"type": "object", "properties": {
            "todos": {"type": "array", "description": "The complete task list snapshot (replaces the previous list).", "items": {"type": "object", "properties": {
                "content": {"type": "string", "description": "Brief description of the task"},
                "status": {"type": "string", "description": "Current status: pending, in_progress, completed, cancelled"},
                "priority": {"type": "string", "description": "Priority: high, medium, low"},
            }, "required": ["content", "status", "priority"]}},
        }, "required": ["todos"]})),
        ("update_agent_graph", "Update Agent Graph", "Adjust the durable agent graph. operation=add_work schedules work (the host runs it as subagents; dependencies via input_ids), operation=stop stops work, operation=finish closes the graph with the selected committed result ids. The first add_work opens a graph.".into(), json!({"type": "object", "properties": {
            "operation": {"anyOf": [{"type": "string", "const": "add_work"}, {"type": "string", "const": "stop"}, {"type": "string", "const": "finish"}], "description": "Always set it. Inferred from the one of add_work / stop / finish you set when omitted."},
            "task": {"type": "string", "description": "One-line title of the overall graph (first add_work only)."},
            "add_work": {"type": "array", "items": work, "maxItems": MAX_ADD_WORK},
            "stop": {"type": "array", "maxItems": MAX_STOP, "items": {"type": "object", "properties": {"target_id": {"type": "string", "description": "Work id or operator id."}, "reason": {"type": "string"}}, "required": ["target_id", "reason"]}},
            "finish": {"type": "object", "properties": {"result_ids": {"type": "array", "items": {"type": "string"}, "description": "Committed result ids (r-<work_id>) selected as the final result.", "maxItems": MAX_RESULT_IDS}, "reason": {"type": "string"}}, "required": ["result_ids", "reason"]},
        }})),
        ("view_agent_graph", "View Agent Graph", "Inspect the durable agent graph: work status, waits, operators, failures, committed result ids, and (history:true) earlier graphs.".into(), json!({"type": "object", "properties": {
            "graph_id": {"type": "string", "description": "An earlier graph id; default the current graph."},
            "history": {"type": "boolean", "description": "Include the list of earlier graphs."},
        }})),
        ("yield_agent_graph", "Yield Agent Graph", "End this supervisor turn successfully while scheduled graph work continues. Call this after the current scheduling wave has no immediate decision; do not poll, sleep, or emit a waiting message. The host will start a new supervisor turn at the next durable graph checkpoint. This does not finish or close the graph.".into(), json!({"type": "object", "properties": {"reason": {"type": "string", "description": "Short note on what you are waiting for."}}})),
        ("agent_output", "Agent Output", "Read a graph work item's committed result (or its failure / partial output) by result id (r-<work_id>) or work id. Use for final results you rely on or narrow failure diagnosis.".into(), json!({"type": "object", "properties": {
            "id": {"type": "string", "description": "Result id (r-<work_id>) or work id."},
            "graph_id": {"type": "string", "description": "An earlier graph id; default: current graph, then history."},
            "max_bytes": {"type": "number", "description": "Default 32768, max 131072."},
        }, "required": ["id"]})),
        ("agent_swarm_status", "Agent Swarm Status", "Compact swarm status: per-item status and result ids (never payload), plus running | needs_attention | settled. Read results with agent_output.".into(), json!({"type": "object", "properties": {}})),
    ]
}

/// `/<mode>` descriptions.
#[must_use]
pub fn mode_description(mode: &str) -> &'static str {
    match mode {
        "goal" => {
            "One objective until verifiably true. Use: medium-complex single deliverables. Cost/success: pareto per success. (<prompt> | status | stop | resume | clear)"
        }
        "ralph" => {
            "Checklist grind over iterations. Use: enumerable chores, repo-scale plans. Cost/success: low cost, solid success. (start <name> | stop | status | resume | clear | <prompt>)"
        }
        "swarm" => {
            "Parallel fan-out + one synthesis on the agent graph. Use: complex decomposable work. Cost/success: higher cost, high coverage. (<prompt> | status | history | stop | resume | clear)"
        }
        _ => {
            "Dependent multi-step work on the durable agent graph. Use: later steps need earlier results. Cost/success: highest; run when the shape demands it. (<prompt> | status | history | stop | resume | clear)"
        }
    }
}

/// Command completions `(value, label, description)`.
#[must_use]
pub fn completions(mode: &str, prefix: &str) -> Vec<(String, String, String)> {
    let mut subs: Vec<(&str, &str, &str)> = if mode == "ralph" {
        vec![
            ("start ", "start <name>", "begin a loop with a task file"),
            ("stop", "stop", "park the active loop"),
            ("status", "status", "loop + task-file progress"),
            ("resume", "resume", "reactivate the parked loop"),
            ("clear", "clear", "drop the parked loop"),
        ]
    } else {
        vec![
            ("status", "status", "show owner + mode snapshot"),
            ("stop", "stop", "end the active owner (terminal)"),
            ("resume", "resume", "reactivate the parked owner"),
            ("clear", "clear", "drop the parked owner"),
        ]
    };
    if mode == "swarm" || mode == "graph" {
        subs.push((
            "history",
            "history",
            "past agent graphs and their selected results",
        ));
    }
    let p = prefix.trim();
    let mut out: Vec<(String, String, String)> = Vec::new();
    if p.is_empty() {
        out.push((
            String::new(),
            "<prompt>".into(),
            format!(
                "run one turn in {} mode",
                if mode == "ralph" { "ralph" } else { "this" }
            ),
        ));
    }
    out.extend(
        subs.into_iter()
            .filter(|s| s.0.trim().starts_with(p))
            .map(|(v, l, d)| (v.to_owned(), l.to_owned(), d.to_owned())),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas() {
        let t = all();
        assert_eq!(t.len(), 11);
        let g = t.iter().find(|x| x.0 == "update_goal").unwrap();
        assert_eq!(g.3["required"], json!(["mode"]));
        let u = t.iter().find(|x| x.0 == "update_agent_graph").unwrap();
        assert_eq!(u.3["properties"]["add_work"]["maxItems"], 32);
        assert_eq!(completions("graph", "his")[0].0, "history");
        assert_eq!(completions("goal", "")[0].1, "<prompt>");
        assert!(mode_description("ralph").starts_with("Checklist grind"));
    }
}
