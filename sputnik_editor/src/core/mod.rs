//! The editor model: text, a selection, the actions that change them, and
//! the pipeline that turns the result into something drawable.
//!
//! Nothing here knows what it is being drawn with. There is no `use iced::`
//! anywhere below this point, and there must not be — that property is
//! what makes the same core usable from a terminal front-end, and it is
//! worth checking mechanically:
//!
//! ```text
//! grep -rn iced sputnik_editor/src/core/ | grep -v '//'   # prints nothing
//! ```
//!
//! The pieces fit together in one direction only:
//!
//! ```text
//!   Action ──► Document ──► Text        what changed, and to what
//!                  │
//!                  └──────► Layout      "which row is up from here?"
//!
//!   Text ──► Layer ──► Row ──► Mapping  what to draw, and where it came from
//! ```
//!
//! A host drives the left half by turning input into [`Action`]s; a widget
//! drives the right half once per frame. Keys, mouse buttons and pixels
//! never reach this far.

mod action;
mod document;
mod layout;
mod render;
mod selection;
mod style;
mod text;

pub use action::{Action, Edit, Motion};
pub use document::Document;
pub use layout::{Layout, LogicalLayout};
pub use render::{Fingerprint, Fragment, Layer, Mapping, Plain, Row, Stack};
pub use selection::Selection;
pub use style::{Color, Style};
pub use text::{Chunks, Text};
