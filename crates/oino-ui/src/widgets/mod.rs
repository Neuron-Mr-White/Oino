//! ratatui widget nodes (§4.5.1).

pub mod barchart;
pub mod block;
pub mod chart;
pub mod gauge;
pub mod list;
pub mod paragraph;
pub mod scrollbar;
pub mod sparkline;
pub mod table;
pub mod tabs;

/// Compatibility alias: the table keeps the same state shape as a list
/// (Appendix A `Table.state` → `ListState`).
pub use list::TableState;
