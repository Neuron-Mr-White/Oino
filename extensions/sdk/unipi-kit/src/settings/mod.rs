//! Settings: schema builder ([`schema`]) and resolved values ([`values`]).
//!
//! ```ignore
//! let ns = Namespace::new("hints", "Hints", "unipi.core")
//!     .defaults(json!({"enabled": true}))
//!     .section(Section::new("hints", "Unicrab hints").field(Field::boolean("enabled", "Hints")));
//! cx.settings_namespace(&ns.to_json());
//! store.declare("hints", ns.defaults.clone());
//! store.load(cx.settings());
//! ```

pub mod schema;
pub mod values;

pub use schema::{Choice, Field, Kind, Namespace, Scope, Section, Unit, choice, plain};
pub use values::{Store, Values, WriteScope, deep_merge, get_field, set_field};
