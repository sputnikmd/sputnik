//! A text editor built as four independent pieces.
//!
//! ```text
//!   core::Text        pluggable storage: the bytes, and nothing else
//!   core::Document    text plus a selection, changed only by Actions
//!   core::Layer       stackable text -> drawable-row transformations
//!   widget::TextEditor   draws the rows, reports where the mouse landed
//! ```
//!
//! They depend on each other in one direction only, and [`core`] never
//! mentions iced. A terminal front-end reuses it as it stands, supplying
//! its own [`core::Layout`] for the motions that depend on wrapping.
//!
//! Input handling belongs to the host. The widget publishes an
//! [`Interaction`] — a resolved position, no more — and the host decides
//! what it means by building an [`Action`]; keys never reach the widget at
//! all. Swapping that mapping changes the entire control scheme.
//!
//! # Examples
//!
//! ```
//! use sputnik_editor::{Action, Edit, Editor, Motion};
//!
//! let mut editor = Editor::<String>::from_str("hello world");
//!
//! editor.perform(Action::Move(Motion::To(5)));
//! editor.perform(Action::Select(Motion::DocumentEnd));
//! editor.perform(Action::Edit(Edit::Backspace));
//!
//! assert_eq!(editor.text(), "hello");
//! ```
#![deny(missing_docs)]

pub mod core;
mod editor;
mod visual;
pub mod widget;

pub use core::{
    Action, Color, Document, Edit, Fragment, Layer, Motion, Plain, Row, Selection, Stack, Style,
    Text,
};
pub use editor::{Editor, Viewport};
pub use visual::VisualLayout;
pub use widget::{Interaction, TextEditor};
