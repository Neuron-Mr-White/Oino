//! WP-U2: regenerates `crates/oino-ui/tests/layout_vectors.json` from ratatui
//! 0.29 itself (ui-protocol §4.3, §12.4, §15). The file is the layout truth for
//! both renderers: the TUI already solves with ratatui, and the app's kiwi
//! solver is checked against the same vectors (±1 cell tolerance).
//!
//! Running this test re-derives every vector and compares it byte-for-byte
//! with the checked-in file, so a rerun is a no-op diff. If the matrix or the
//! ratatui version changes, the test rewrites the file and fails — review the
//! diff and commit the new vectors.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a generator test asserts by unwrapping"
)]

use std::fs;

use oino_ui::layout::{Constraint as UiConstraint, Direction, Flex as UiFlex, Margin as UiMargin};
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use serde::Serialize;

/// Cross-axis extent of the solved area; the main axis is `size`.
const CROSS: u16 = 100;
/// ratatui version the vectors were generated with.
const RATATUI_VERSION: &str = "0.29";
/// Checked-in vector file, written on drift.
const TARGET: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../oino-ui/tests/layout_vectors.json"
);

/// One solved layout case.
struct Case {
    name: String,
    direction: Direction,
    size: u16,
    constraints: Vec<UiConstraint>,
    flex: UiFlex,
    spacing: u16,
    margin: Option<UiMargin>,
}

/// A serialized vector (field order = JSON key order).
#[derive(Serialize)]
struct Vector {
    name: String,
    direction: Direction,
    size: u16,
    constraints: Vec<UiConstraint>,
    flex: UiFlex,
    spacing: u16,
    margin: Option<UiMargin>,
    expected: Vec<[u16; 4]>,
}

#[derive(Serialize)]
struct VectorFile {
    generator: &'static str,
    ratatui: &'static str,
    cross: u16,
    vectors: Vec<Vector>,
}

/// The constraint universe (§12.4: every constraint alone and paired with
/// every other, incl. the degenerate forms).
fn constraint_pool() -> Vec<UiConstraint> {
    vec![
        UiConstraint::Length(20),
        UiConstraint::Percentage(50),
        UiConstraint::Percentage(0),
        UiConstraint::Percentage(100),
        UiConstraint::Ratio(1, 4),
        UiConstraint::Ratio(1, 0),
        UiConstraint::Min(20),
        UiConstraint::Max(20),
        UiConstraint::Fill(1),
        UiConstraint::Fill(0),
        UiConstraint::Fill(3),
    ]
}

fn cons_tag(constraint: &UiConstraint) -> String {
    match *constraint {
        UiConstraint::Length(n) => format!("L{n}"),
        UiConstraint::Percentage(n) => format!("P{n}"),
        UiConstraint::Ratio(a, b) => format!("R{a}-{b}"),
        UiConstraint::Min(n) => format!("Min{n}"),
        UiConstraint::Max(n) => format!("Max{n}"),
        UiConstraint::Fill(w) => format!("F{w}"),
    }
}

fn flex_tag(flex: UiFlex) -> &'static str {
    match flex {
        UiFlex::Legacy => "legacy",
        UiFlex::Start => "start",
        UiFlex::End => "end",
        UiFlex::Center => "center",
        UiFlex::SpaceBetween => "sbetween",
        UiFlex::SpaceAround => "saround",
    }
}

fn margin_tag(margin: Option<UiMargin>) -> String {
    match margin {
        None => "0".to_owned(),
        Some(UiMargin::Uniform(n)) => format!("u{n}"),
        Some(UiMargin::Axes {
            horizontal,
            vertical,
        }) => format!("h{horizontal}v{vertical}"),
    }
}

fn case(
    direction: Direction,
    constraints: Vec<UiConstraint>,
    size: u16,
    flex: UiFlex,
    spacing: u16,
    margin: Option<UiMargin>,
) -> Case {
    let cons = constraints
        .iter()
        .map(cons_tag)
        .collect::<Vec<_>>()
        .join("+");
    Case {
        name: format!(
            "{}[{cons}]sz{size}-{}-sp{spacing}-m{}",
            if direction == Direction::Vertical {
                "v"
            } else {
                "h"
            },
            flex_tag(flex),
            margin_tag(margin),
        ),
        direction,
        size,
        constraints,
        flex,
        spacing,
        margin,
    }
}

/// Solves one case with ratatui and records every child `Rect`.
fn solve(case_: &Case) -> Vec<[u16; 4]> {
    let area = match case_.direction {
        Direction::Vertical => Rect::new(0, 0, CROSS, case_.size),
        Direction::Horizontal => Rect::new(0, 0, case_.size, CROSS),
    };
    let (area, uniform_margin) = match case_.margin {
        // ratatui's `margin` is uniform; an axis-split inset shrinks the area
        // first, which is what `Layout::margin` does internally.
        Some(UiMargin::Axes {
            horizontal,
            vertical,
        }) => (
            Rect::new(
                area.x.saturating_add(horizontal),
                area.y.saturating_add(vertical),
                area.width.saturating_sub(horizontal.saturating_mul(2)),
                area.height.saturating_sub(vertical.saturating_mul(2)),
            ),
            0,
        ),
        Some(UiMargin::Uniform(n)) => (area, n),
        None => (area, 0),
    };
    let constraints: Vec<Constraint> = case_
        .constraints
        .iter()
        .map(|c| match *c {
            UiConstraint::Length(n) => Constraint::Length(n),
            UiConstraint::Percentage(n) => Constraint::Percentage(n),
            UiConstraint::Ratio(a, b) => Constraint::Ratio(a, b),
            UiConstraint::Min(n) => Constraint::Min(n),
            UiConstraint::Max(n) => Constraint::Max(n),
            UiConstraint::Fill(w) => Constraint::Fill(w),
        })
        .collect();
    let layout = match case_.direction {
        Direction::Vertical => Layout::vertical(constraints),
        Direction::Horizontal => Layout::horizontal(constraints),
    }
    .flex(match case_.flex {
        UiFlex::Legacy => Flex::Legacy,
        UiFlex::Start => Flex::Start,
        UiFlex::End => Flex::End,
        UiFlex::Center => Flex::Center,
        UiFlex::SpaceBetween => Flex::SpaceBetween,
        UiFlex::SpaceAround => Flex::SpaceAround,
    })
    .spacing(case_.spacing)
    .margin(uniform_margin);
    layout
        .split(area)
        .iter()
        .map(|r| [r.x, r.y, r.width, r.height])
        .collect()
}

/// The ≥ 200-case matrix (§12.4 WP-U2 row).
fn cases() -> Vec<Case> {
    let mut out = base_matrix();
    out.extend(flex_sweep());
    out.extend(spacing_sweep());
    out.extend(margin_sweep());
    out.extend(size_sweep());
    out.extend(special_shapes());
    out.extend(horizontal_mirrors());

    // The sweeps overlap the base matrix (same set/size/flex/spacing/margin);
    // keep the first occurrence so vector names stay unique.
    let mut seen = std::collections::HashSet::new();
    out.retain(|case_| seen.insert(case_.name.clone()));
    out
}

/// Every constraint alone and paired with every other, at two sizes.
fn base_matrix() -> Vec<Case> {
    let pool = constraint_pool();
    let mut out = Vec::new();
    for (index, first) in pool.iter().enumerate() {
        for size in [24, 80] {
            out.push(case(
                Direction::Vertical,
                vec![*first],
                size,
                UiFlex::Start,
                0,
                None,
            ));
        }
        for second in &pool[index + 1..] {
            for size in [24, 80] {
                out.push(case(
                    Direction::Vertical,
                    vec![*first, *second],
                    size,
                    UiFlex::Start,
                    0,
                    None,
                ));
            }
        }
    }
    out
}

/// Every flex on representative sets.
fn flex_sweep() -> Vec<Case> {
    let sets: &[&[UiConstraint]] = &[
        &[UiConstraint::Length(20), UiConstraint::Length(20)],
        &[UiConstraint::Length(20), UiConstraint::Fill(1)],
        &[UiConstraint::Min(20), UiConstraint::Max(20)],
        &[UiConstraint::Fill(1), UiConstraint::Fill(1)],
        &[UiConstraint::Max(20), UiConstraint::Max(20)],
        &[UiConstraint::Percentage(50), UiConstraint::Fill(1)],
    ];
    let flexes = [
        UiFlex::Legacy,
        UiFlex::Start,
        UiFlex::End,
        UiFlex::Center,
        UiFlex::SpaceBetween,
        UiFlex::SpaceAround,
    ];
    let mut out = Vec::new();
    for set in sets {
        for flex in flexes {
            out.push(case(Direction::Vertical, set.to_vec(), 80, flex, 0, None));
        }
    }
    out
}

/// Spacing 1 and 3 on representative sets.
fn spacing_sweep() -> Vec<Case> {
    let sets: &[&[UiConstraint]] = &[
        &[UiConstraint::Length(20), UiConstraint::Length(20)],
        &[UiConstraint::Fill(1), UiConstraint::Fill(1)],
        &[
            UiConstraint::Length(10),
            UiConstraint::Fill(1),
            UiConstraint::Length(10),
        ],
    ];
    let mut out = Vec::new();
    for set in sets {
        for spacing in [1, 3] {
            for size in [40, 80] {
                out.push(case(
                    Direction::Vertical,
                    set.to_vec(),
                    size,
                    UiFlex::Start,
                    spacing,
                    None,
                ));
            }
        }
    }
    out
}

/// Margins 0/1 and axis-split insets.
fn margin_sweep() -> Vec<Case> {
    let sets: &[&[UiConstraint]] = &[
        &[UiConstraint::Fill(1), UiConstraint::Fill(1)],
        &[UiConstraint::Length(20), UiConstraint::Length(20)],
    ];
    let mut out = Vec::new();
    for set in sets {
        for margin in [
            Some(UiMargin::Uniform(1)),
            Some(UiMargin::Axes {
                horizontal: 1,
                vertical: 0,
            }),
            Some(UiMargin::Axes {
                horizontal: 2,
                vertical: 1,
            }),
        ] {
            out.push(case(
                Direction::Vertical,
                set.to_vec(),
                40,
                UiFlex::Start,
                0,
                margin,
            ));
        }
        out.push(case(
            Direction::Horizontal,
            set.to_vec(),
            40,
            UiFlex::Start,
            0,
            Some(UiMargin::Axes {
                horizontal: 1,
                vertical: 0,
            }),
        ));
    }
    out
}

/// Edge sizes 0/1/7/200.
fn size_sweep() -> Vec<Case> {
    let sets: &[&[UiConstraint]] = &[
        &[UiConstraint::Length(20), UiConstraint::Length(20)],
        &[UiConstraint::Fill(1), UiConstraint::Fill(1)],
        &[UiConstraint::Min(20), UiConstraint::Max(20)],
    ];
    let mut out = Vec::new();
    for set in sets {
        for size in [0, 1, 7, 200] {
            out.push(case(
                Direction::Vertical,
                set.to_vec(),
                size,
                UiFlex::Start,
                0,
                None,
            ));
        }
    }
    out
}

/// Special shapes from the spec table (§12.4).
fn special_shapes() -> Vec<Case> {
    vec![
        case(
            Direction::Vertical,
            vec![
                UiConstraint::Length(20),
                UiConstraint::Length(20),
                UiConstraint::Length(20),
            ],
            80,
            UiFlex::Legacy,
            0,
            None,
        ),
        case(
            Direction::Vertical,
            vec![
                UiConstraint::Length(3),
                UiConstraint::Fill(1),
                UiConstraint::Length(1),
            ],
            24,
            UiFlex::Start,
            0,
            None,
        ),
        case(
            Direction::Vertical,
            vec![
                UiConstraint::Fill(1),
                UiConstraint::Fill(2),
                UiConstraint::Fill(3),
            ],
            50,
            UiFlex::Start,
            0,
            None,
        ),
        case(
            Direction::Vertical,
            vec![UiConstraint::Ratio(1, 4); 4],
            50,
            UiFlex::Start,
            0,
            None,
        ),
        case(
            Direction::Vertical,
            vec![
                UiConstraint::Fill(1),
                UiConstraint::Percentage(50),
                UiConstraint::Fill(1),
            ],
            50,
            UiFlex::Start,
            0,
            None,
        ),
        case(
            Direction::Vertical,
            vec![UiConstraint::Length(100), UiConstraint::Length(100)],
            80,
            UiFlex::Start,
            0,
            None,
        ),
        case(
            Direction::Vertical,
            vec![UiConstraint::Min(50), UiConstraint::Min(50)],
            80,
            UiFlex::Start,
            0,
            None,
        ),
    ]
}

/// Horizontal mirrors of representative sets.
fn horizontal_mirrors() -> Vec<Case> {
    let sets: &[&[UiConstraint]] = &[
        &[UiConstraint::Length(20), UiConstraint::Fill(1)],
        &[UiConstraint::Fill(1), UiConstraint::Fill(1)],
        &[
            UiConstraint::Length(3),
            UiConstraint::Fill(1),
            UiConstraint::Length(1),
        ],
    ];
    sets.iter()
        .map(|set| {
            case(
                Direction::Horizontal,
                set.to_vec(),
                80,
                UiFlex::Start,
                0,
                None,
            )
        })
        .collect()
}

#[test]
fn regenerate_layout_vectors() {
    let cases = cases();
    assert!(
        cases.len() >= 200,
        "need ≥ 200 cases, built {}",
        cases.len()
    );
    let vectors: Vec<Vector> = cases
        .iter()
        .map(|case_| Vector {
            name: case_.name.clone(),
            direction: case_.direction,
            size: case_.size,
            constraints: case_.constraints.clone(),
            flex: case_.flex,
            spacing: case_.spacing,
            margin: case_.margin,
            expected: solve(case_),
        })
        .collect();
    let file = VectorFile {
        generator: "crates/oino-tui/tests/gen_layout_vectors.rs",
        ratatui: RATATUI_VERSION,
        cross: CROSS,
        vectors,
    };
    let rendered = serde_json::to_string_pretty(&file).expect("vectors serialize") + "\n";

    let existing = fs::read_to_string(TARGET).unwrap_or_default();
    if existing != rendered {
        fs::write(TARGET, &rendered).expect("writes layout_vectors.json");
        panic!(
            "layout_vectors.json drifted from ratatui {RATATUI_VERSION}; \
             the file was regenerated — review `git diff` on it and commit"
        );
    }
}
