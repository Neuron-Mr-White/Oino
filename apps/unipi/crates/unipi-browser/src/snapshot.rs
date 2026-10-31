//! Accessibility snapshot (browser-control.md §7.1): CDP `Accessibility.getFullAXTree` per
//! frame, merged at iframe owners, rendered in Playwright `ariaSnapshot` "ai" style with
//! `[ref=eN]` on interactive nodes. Pure: CDP fetching lives in `page.rs`.

use std::collections::HashMap;
use std::fmt::Write as _;

use serde_json::Value;

/// Roles that always get a ref.
const INTERACTIVE: &[&str] = &[
    "link",
    "button",
    "textbox",
    "searchbox",
    "combobox",
    "listbox",
    "option",
    "checkbox",
    "radio",
    "switch",
    "slider",
    "spinbutton",
    "tab",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "treeitem",
    "textField",
    "TextField",
];

/// Anonymous containers flattened into their parent.
const FLATTEN: &[&str] = &[
    "generic",
    "none",
    "presentation",
    "group",
    "Section",
    "LayoutTable",
    "LayoutTableRow",
    "LayoutTableCell",
    "InlineTextBox",
    "LineBreak",
    "Div",
    "Pre",
    "Abbr",
    "Ruby",
    "Canvas",
    "LabelText",
    "ListMarker",
    "superscript",
    "subscript",
    "emphasis",
    "strong",
    "Mark",
    "time",
    "code",
];

/// One AX node (tolerant parse of `Accessibility.AXNode`).
#[derive(Clone, Debug, Default)]
pub(crate) struct AxNode {
    pub(crate) id: String,
    pub(crate) ignored: bool,
    pub(crate) role: String,
    pub(crate) name: String,
    pub(crate) value: Option<String>,
    pub(crate) props: HashMap<String, Value>,
    pub(crate) children: Vec<String>,
    pub(crate) backend: Option<i64>,
}

fn ax_value(v: Option<&Value>) -> Option<Value> {
    v.and_then(|v| v.get("value")).cloned()
}

fn text_of(v: Option<&Value>) -> String {
    match ax_value(v) {
        Some(Value::String(s)) => s,
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

impl AxNode {
    /// Parses one node.
    pub(crate) fn parse(v: &Value) -> Option<Self> {
        let id = v.get("nodeId")?.as_str()?.to_owned();
        let props = v
            .get("properties")
            .and_then(Value::as_array)
            .map(|ps| {
                ps.iter()
                    .filter_map(|p| {
                        Some((
                            p.get("name")?.as_str()?.to_owned(),
                            ax_value(p.get("value")).unwrap_or(Value::Null),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            id,
            ignored: v.get("ignored").and_then(Value::as_bool).unwrap_or(false),
            role: text_of(v.get("role")),
            name: text_of(v.get("name")),
            value: ax_value(v.get("value")).map(|x| match x {
                Value::String(s) => s,
                other => other.to_string(),
            }),
            props,
            children: v
                .get("childIds")
                .and_then(Value::as_array)
                .map(|c| {
                    c.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            backend: v.get("backendDOMNodeId").and_then(Value::as_i64),
        })
    }

    fn prop_bool(&self, name: &str) -> Option<bool> {
        self.props.get(name).and_then(Value::as_bool)
    }
}

/// The AX tree of one frame.
#[derive(Clone, Debug, Default)]
pub(crate) struct FrameTree {
    /// CDP session the nodes belong to.
    pub(crate) session: String,
    /// Frame id.
    pub(crate) frame_id: String,
    /// Nodes in CDP order (root first).
    pub(crate) nodes: Vec<AxNode>,
    /// OOPIF owner chain to the top-level page: `(parent session, owner backend id)`.
    pub(crate) chain: Vec<(String, i64)>,
}

/// Every frame of a page plus which iframe owner hosts which frame.
#[derive(Clone, Debug, Default)]
pub(crate) struct Forest {
    /// Frames; index 0 is the main frame.
    pub(crate) frames: Vec<FrameTree>,
    /// `(session, owner backend id)` → frame index.
    pub(crate) owners: HashMap<(String, i64), usize>,
}

/// Where a ref points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RefEntry {
    /// CDP session.
    pub(crate) session: String,
    /// `backendDOMNodeId`.
    pub(crate) backend: i64,
    /// Frame.
    pub(crate) frame_id: String,
    /// OOPIF chain (see [`FrameTree::chain`]).
    pub(crate) chain: Vec<(String, i64)>,
    /// Snapshot generation that last saw it.
    pub(crate) snapshot: u64,
    /// Role.
    pub(crate) role: String,
    /// Accessible name.
    pub(crate) name: String,
}

/// Per-tab ref table; refs are monotonic and stable for the same node across snapshots.
#[derive(Debug, Default)]
pub(crate) struct RefMap {
    next: u64,
    by_ref: HashMap<String, RefEntry>,
    by_node: HashMap<(String, i64), String>,
}

impl RefMap {
    /// Looks up a ref.
    pub(crate) fn get(&self, r: &str) -> Option<&RefEntry> {
        self.by_ref.get(r)
    }

    fn assign(&mut self, entry: RefEntry) -> String {
        let key = (entry.session.clone(), entry.backend);
        if let Some(r) = self.by_node.get(&key) {
            let r = r.clone();
            self.by_ref.insert(r.clone(), entry);
            return r;
        }
        self.next += 1;
        let r = format!("e{}", self.next);
        self.by_node.insert(key, r.clone());
        self.by_ref.insert(r.clone(), entry);
        r
    }

    /// Forgets everything (navigation in a new document keeps numbering monotonic).
    pub(crate) fn clear(&mut self) {
        self.by_ref.clear();
        self.by_node.clear();
    }
}

/// Rendering options.
#[derive(Clone, Debug)]
pub(crate) struct RenderOptions {
    /// Character budget.
    pub(crate) max_chars: usize,
    /// Snapshot generation.
    pub(crate) snapshot: u64,
    /// Limit to this ref's subtree.
    pub(crate) scope: Option<RefEntry>,
}

/// Renders `forest`, assigning refs in `refs`. Returns `(text, truncated)`.
pub(crate) fn render(forest: &Forest, refs: &mut RefMap, opts: &RenderOptions) -> (String, bool) {
    let mut r = Renderer {
        forest,
        refs,
        opts,
        out: String::new(),
        lines: 0,
    };
    let Some(main) = forest.frames.first() else {
        return (String::new(), false);
    };
    let index: Vec<HashMap<&str, &AxNode>> = forest
        .frames
        .iter()
        .map(|f| f.nodes.iter().map(|n| (n.id.as_str(), n)).collect())
        .collect();
    let start = match &opts.scope {
        Some(scope) => forest.frames.iter().enumerate().find_map(|(fi, f)| {
            (f.session == scope.session)
                .then(|| {
                    f.nodes
                        .iter()
                        .find(|n| n.backend == Some(scope.backend))
                        .map(|n| (fi, n.id.as_str()))
                })
                .flatten()
        }),
        None => main.nodes.first().map(|n| (0, n.id.as_str())),
    };
    if let Some((fi, id)) = start {
        r.node(&index, fi, id, 0, opts.scope.is_none());
    }
    let mut text = r.out;
    let truncated = text.len() > opts.max_chars;
    if truncated {
        let mut cut = opts.max_chars;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        if let Some(nl) = text[..cut].rfind('\n') {
            cut = nl + 1;
        }
        text.truncate(cut);
    }
    (text, truncated)
}

struct Renderer<'a> {
    forest: &'a Forest,
    refs: &'a mut RefMap,
    opts: &'a RenderOptions,
    out: String,
    lines: usize,
}

fn clip(s: &str, max: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= max {
        return s;
    }
    let mut t: String = s.chars().take(max).collect();
    t.push('…');
    t
}

fn quote(s: &str) -> String {
    format!("\"{}\"", clip(s, 200).replace('"', "\\\""))
}

impl Renderer<'_> {
    /// Collected plain text of `id`'s subtree (for name-dedupe and text-only containers).
    fn subtree_text(index: &[HashMap<&str, &AxNode>], fi: usize, id: &str) -> String {
        let Some(n) = index[fi].get(id) else {
            return String::new();
        };
        if n.role == "StaticText" {
            return n.name.clone();
        }
        n.children
            .iter()
            .map(|c| Self::subtree_text(index, fi, c))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn is_text_only(index: &[HashMap<&str, &AxNode>], fi: usize, n: &AxNode) -> bool {
        n.children.iter().all(|c| {
            index[fi].get(c.as_str()).is_none_or(|k| {
                k.role == "StaticText"
                    || k.role == "InlineTextBox"
                    || k.role == "LineBreak"
                    || ((k.ignored || FLATTEN.contains(&k.role.as_str()))
                        && k.name.is_empty()
                        && Self::is_text_only(index, fi, k))
            })
        })
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one node rendering rule set, kept together for readability"
    )]
    fn node(
        &mut self,
        index: &[HashMap<&str, &AxNode>],
        fi: usize,
        id: &str,
        depth: usize,
        root: bool,
    ) {
        let Some(n) = index[fi].get(id).copied() else {
            return;
        };
        if self.out.len() > self.opts.max_chars + 1024 {
            return;
        }
        let role = n.role.as_str();
        let page_root = role == "RootWebArea" || role == "WebArea";
        if page_root && (root || depth == 0 || fi > 0) {
            for c in &n.children {
                self.node(index, fi, c, depth, false);
            }
            return;
        }
        if role == "StaticText" {
            let t = clip(&n.name, 200);
            if !t.is_empty() {
                self.line(depth, &format!("- text: {t}"));
            }
            return;
        }
        if role == "InlineTextBox" || role == "LineBreak" {
            return;
        }
        let anonymous = n.name.trim().is_empty();
        if n.ignored || (FLATTEN.contains(&role) && anonymous && !Self::wants_ref(n)) {
            for c in &n.children {
                self.node(index, fi, c, depth, false);
            }
            return;
        }
        if (role == "image" || role == "img") && anonymous {
            return;
        }
        let mut head = format!("- {}", role_name(role));
        if !anonymous {
            head.push(' ');
            head.push_str(&quote(&n.name));
        }
        if Self::wants_ref(n)
            && let Some(backend) = n.backend
        {
            let frame = &self.forest.frames[fi];
            let r = self.refs.assign(RefEntry {
                session: frame.session.clone(),
                backend,
                frame_id: frame.frame_id.clone(),
                chain: frame.chain.clone(),
                snapshot: self.opts.snapshot,
                role: role.to_owned(),
                name: clip(&n.name, 80),
            });
            let _ = write!(head, " [ref={r}]");
        }
        for (prop, label) in [
            ("checked", "checked"),
            ("pressed", "pressed"),
            ("selected", "selected"),
            ("expanded", "expanded"),
        ] {
            match n.props.get(prop) {
                Some(Value::Bool(true)) => {
                    let _ = write!(head, " [{label}]");
                }
                Some(Value::Bool(false)) if prop == "checked" || prop == "expanded" => {
                    let _ = write!(head, " [{label}=false]");
                }
                Some(Value::String(s)) if s == "true" => {
                    let _ = write!(head, " [{label}]");
                }
                Some(Value::String(s)) if s == "mixed" => {
                    let _ = write!(head, " [{label}=mixed]");
                }
                Some(Value::String(s)) if s == "false" && prop == "checked" => {
                    let _ = write!(head, " [{label}=false]");
                }
                _ => {}
            }
        }
        for flag in ["disabled", "required", "invalid"] {
            if n.prop_bool(flag) == Some(true)
                || n.props
                    .get(flag)
                    .and_then(Value::as_str)
                    .is_some_and(|s| s != "false")
            {
                let _ = write!(head, " [{flag}]");
            }
        }
        if let Some(level) = n.props.get("level").and_then(Value::as_i64) {
            let _ = write!(head, " [level={level}]");
        }
        let editable = matches!(
            role,
            "textbox" | "searchbox" | "combobox" | "spinbutton" | "slider" | "textField"
        );
        if editable {
            let v = n.value.clone().unwrap_or_default();
            let masked = n
                .props
                .get("autocomplete")
                .is_some_and(|a| a == "current-password")
                || (!v.is_empty() && v.chars().all(|c| c == '•' || c == '●'));
            let v = if masked {
                "••••".to_owned()
            } else {
                clip(&v, 200)
            };
            let _ = write!(head, ": \"{v}\"");
            self.line(depth, &head);
            return;
        }
        if role == "Iframe" || role == "iframe" {
            let frame = &self.forest.frames[fi];
            let child = n
                .backend
                .and_then(|b| self.forest.owners.get(&(frame.session.clone(), b)).copied());
            if let Some(ci) = child {
                head.push(':');
                self.line(depth, &head);
                if let Some(root) = self.forest.frames[ci].nodes.first() {
                    let rid = root.id.clone();
                    self.node(index, ci, &rid, depth + 1, true);
                }
            } else {
                head.push_str(" (unavailable)");
                self.line(depth, &head);
            }
            return;
        }
        let kids_text = Self::subtree_text(index, fi, &n.id);
        if Self::is_text_only(index, fi, n) {
            let t = clip(&kids_text, 200);
            if !t.is_empty() && clip(&n.name, 200) != t {
                let _ = write!(head, ": {t}");
            }
            self.line(depth, &head);
            return;
        }
        head.push(':');
        self.line(depth, &head);
        let kids: Vec<&String> = n.children.iter().collect();
        let many = kids.len() > 40 && self.out.len() > self.opts.max_chars / 2;
        let shown = if many { 20 } else { kids.len() };
        for c in &kids[..shown] {
            self.node(index, fi, c, depth + 1, false);
        }
        if many {
            self.line(depth + 1, &format!("- ({} more items)", kids.len() - shown));
        }
    }

    fn wants_ref(n: &AxNode) -> bool {
        if n.backend.is_none() || n.prop_bool("disabled") == Some(true) && n.role == "generic" {
            return false;
        }
        INTERACTIVE.contains(&n.role.as_str())
            || (n.prop_bool("focusable") == Some(true)
                && !n.name.trim().is_empty()
                && !matches!(n.role.as_str(), "RootWebArea" | "WebArea" | "Iframe"))
    }

    fn line(&mut self, depth: usize, text: &str) {
        for _ in 0..depth {
            self.out.push_str("  ");
        }
        self.out.push_str(text);
        self.out.push('\n');
        self.lines += 1;
    }
}

fn role_name(role: &str) -> &str {
    match role {
        "textField" | "TextField" => "textbox",
        "img" => "image",
        "Iframe" => "iframe",
        "DescriptionList" => "list",
        "Legend" => "legend",
        "Figcaption" => "caption",
        "LayoutTable" => "table",
        other => other,
    }
}

/// Parses `getFullAXTree` output.
pub(crate) fn parse_nodes(reply: &Value) -> Vec<AxNode> {
    reply
        .get("nodes")
        .and_then(Value::as_array)
        .map(|ns| ns.iter().filter_map(AxNode::parse).collect())
        .unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: &str, role: &str, name: &str, kids: &[&str], backend: i64) -> Value {
        json!({"nodeId": id, "ignored": false, "role": {"type":"role","value": role},
               "name": {"type":"computedString","value": name}, "childIds": kids,
               "backendDOMNodeId": backend, "properties": []})
    }

    fn main_tree() -> Vec<AxNode> {
        let mut nodes = vec![
            node("1", "RootWebArea", "Pricing", &["2", "6"], 1),
            node("2", "banner", "", &["3", "4"], 2),
            node("3", "link", "Example", &["31"], 3),
            node("31", "StaticText", "Example", &[], 31),
            node("4", "generic", "", &["5"], 4),
            node("5", "button", "Monthly", &[], 5),
            node("6", "main", "", &["7", "8", "9", "10"], 6),
            node("7", "heading", "Pricing", &["71"], 7),
            node("71", "StaticText", "Pricing", &[], 71),
            node("8", "paragraph", "", &["81"], 8),
            node("81", "StaticText", "Pick the plan that fits.", &[], 81),
            node("9", "checkbox", "Annual billing", &[], 9),
            node("10", "Iframe", "Stripe", &[], 10),
        ];
        nodes[7]["properties"] = json!([{"name":"level","value":{"type":"integer","value":1}}]);
        nodes[5]["properties"] =
            json!([{"name":"pressed","value":{"type":"tristate","value":"true"}}]);
        nodes[11]["properties"] =
            json!([{"name":"checked","value":{"type":"tristate","value":"false"}}]);
        nodes.iter().filter_map(AxNode::parse).collect()
    }

    #[test]
    fn renders_ai_style_with_refs_and_frames() {
        let child = [
            node("100", "RootWebArea", "Checkout", &["101"], 100),
            node("101", "textbox", "Card number", &[], 101),
        ];
        let mut forest = Forest {
            frames: vec![
                FrameTree {
                    session: "S".into(),
                    frame_id: "F".into(),
                    nodes: main_tree(),
                    chain: vec![],
                },
                FrameTree {
                    session: "C".into(),
                    frame_id: "F2".into(),
                    nodes: child.iter().filter_map(AxNode::parse).collect(),
                    chain: vec![("S".into(), 10)],
                },
            ],
            owners: HashMap::new(),
        };
        forest.owners.insert(("S".into(), 10), 1);
        let mut refs = RefMap::default();
        let opts = RenderOptions {
            max_chars: 24_000,
            snapshot: 1,
            scope: None,
        };
        let (text, truncated) = render(&forest, &mut refs, &opts);
        assert!(!truncated);
        assert_eq!(
            text,
            "- banner:\n  - link \"Example\" [ref=e1]\n  - button \"Monthly\" [ref=e2] [pressed]\n- main:\n  - heading \"Pricing\" [level=1]\n  - paragraph: Pick the plan that fits.\n  - checkbox \"Annual billing\" [ref=e3] [checked=false]\n  - iframe \"Stripe\":\n    - textbox \"Card number\" [ref=e4]: \"\"\n"
        );
        let e4 = refs.get("e4").unwrap();
        assert_eq!((e4.session.as_str(), e4.backend), ("C", 101));
        assert_eq!(e4.chain, vec![("S".to_owned(), 10)]);
        // Stable refs across snapshots, monotonic for new nodes.
        let (again, _) = render(
            &forest,
            &mut refs,
            &RenderOptions {
                snapshot: 2,
                ..opts.clone()
            },
        );
        assert_eq!(again, text);
        forest.owners.clear();
        let (no_frame, _) = render(&forest, &mut refs, &opts);
        assert!(no_frame.contains("- iframe \"Stripe\" (unavailable)"));
        let e1 = refs.get("e1").cloned();
        let scoped = render(
            &forest,
            &mut refs,
            &RenderOptions {
                scope: e1,
                ..opts.clone()
            },
        );
        assert_eq!(scoped.0, "- link \"Example\" [ref=e1]\n");
        let (small, truncated) = render(
            &forest,
            &mut refs,
            &RenderOptions {
                max_chars: 40,
                ..opts
            },
        );
        assert!(truncated);
        assert!(small.len() <= 40);
    }
}
