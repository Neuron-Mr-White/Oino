//! Settings: schema builders ([`schema`] = v1-compatible, [`oss2`] = OSS-2 additions) and
//! resolved values ([`values`]).
//!
//! ```ignore
//! let ns = Namespace::new("hints", "Hints", "unipi.core")
//!     .defaults(json!({"enabled": true}))
//!     .section(Section::new("hints", "Unicrab hints").field(Field::boolean("enabled", "Hints")));
//! cx.settings_namespace(&ns.to_json());
//! store.declare("hints", ns.defaults.clone());
//! store.load(cx.settings());
//! ```

pub mod oss2;
pub mod schema;
pub mod values;

pub use oss2::{
    Cond, FieldScope, FieldType, Restart, SettingsNamespace, SettingsSection, StringFormat,
};
pub use schema::{Choice, Field, Kind, Namespace, Scope, Section, Unit, choice, plain};
pub use values::{Store, Values, WriteScope, deep_merge, get_field, set_field};
