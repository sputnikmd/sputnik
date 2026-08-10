//! What can be asked of a [`Document`](crate::core::Document).
//!
//! This is the whole vocabulary of the editor. Nothing here mentions keys,
//! mouse buttons or pixels: deciding that <kbd>Backspace</kbd> means
//! [`Edit::Backspace`], or that a mouse drag means
//! [`Action::Select`]`(`[`Motion::To`]`(..))`, is the host's job. That is
//! what makes the control scheme replaceable — a modal, vim-style host maps
//! the same actions from an entirely different set of keys, and neither the
//! document nor the widget is any the wiser.
//!
//! # Examples
//!
//! ```
//! use sputnik_editor::core::{Action, Edit, Motion};
//!
//! // A conventional host might build these from keys ...
//! let select_left = Action::Select(Motion::Left);
//! let backspace = Action::Edit(Edit::Backspace);
//! // ... and a modal one from an entirely different set.
//! assert_ne!(select_left, backspace);
//! ```

/// A single, complete change to a document.
///
/// # Examples
///
/// ```
/// use sputnik_editor::core::{Action, Document, LogicalLayout, Motion};
///
/// let mut document = Document::<String>::from_str("hello");
/// document.perform(Action::Move(Motion::DocumentEnd), &LogicalLayout::default());
/// assert_eq!(document.cursor(), 5);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Move the caret, discarding any selection.
    Move(Motion),
    /// Move the caret while holding the selection's anchor in place —
    /// extending or shrinking the selection. This is <kbd>Shift</kbd> +
    /// motion, and also what a mouse drag reports.
    Select(Motion),
    /// Select the entire document.
    SelectAll,
    /// Change the text.
    Edit(Edit),
}

/// Where to move the caret.
///
/// Most variants depend only on the text, so a document resolves them
/// alone. [`Motion::Up`], [`Motion::Down`] and the paging variants depend
/// instead on how the text is rendered — with soft wrap one line is
/// several rows — and are resolved through a
/// [`Layout`](crate::core::Layout).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    /// One character back.
    Left,
    /// One character forward.
    Right,
    /// To the start of the previous word.
    WordLeft,
    /// To the end of the next word.
    WordRight,
    /// Start of the caret's row.
    ///
    /// Resolved from the document's line rather than from what is drawn,
    /// so a [`Layer`](crate::core::Layer) concealing the start of a line
    /// leaves this position inside the concealed text — addressable, but
    /// drawn in the same place as the first visible character. Bind
    /// [`Motion::To`] instead where a stack needs the caret to land on
    /// what is actually drawn.
    RowStart,
    /// End of the caret's row, with the same caveat as
    /// [`Motion::RowStart`].
    RowEnd,
    /// One rendered row up.
    Up,
    /// One rendered row down.
    Down,
    /// One viewport's worth of rows up.
    PageUp,
    /// One viewport's worth of rows down.
    PageDown,
    /// To the very beginning of the document.
    DocumentStart,
    /// To the very end of the document.
    DocumentEnd,
    /// An already-resolved byte position.
    ///
    /// The escape hatch for anything only the owner of the rendered layout
    /// can work out — above all a mouse click, which starts life as a pixel
    /// coordinate and becomes a position in the widget that shaped the
    /// glyphs under it. Out-of-range and mid-character values are snapped,
    /// so a stale position is harmless.
    To(usize),
}

/// A change to the text.
///
/// Every variant replaces the selection when there is one.
///
/// [`Edit::Enter`] and [`Edit::Tab`] stay separate from [`Edit::Insert`]
/// even though both could be expressed as inserting a character: keeping
/// them distinct is what lets the document own newline and indentation
/// policy — auto-indent, tabs against spaces — instead of scattering it
/// across every host that binds a key to them.
///
/// # Examples
///
/// ```
/// use sputnik_editor::core::{Action, Document, Edit, LogicalLayout, Selection};
///
/// let mut document = Document::<String>::from_str("hello world");
/// document.set_selection(Selection::new(0, 6));
/// document.perform(Action::Edit(Edit::Insert(',')), &LogicalLayout::default());
/// assert_eq!(document.text(), ",world");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// Type a character, replacing the selection if there is one.
    Insert(char),
    /// Insert arbitrary text, replacing the selection if there is one.
    Paste(String),
    /// Break the line.
    Enter,
    /// Indent by one tab stop.
    Tab,
    /// Delete the selection, or the character before the caret.
    Backspace,
    /// Delete the selection, or the character after the caret.
    Delete,
}

/// Shorthand for [`Action::Move`].
///
/// ```
/// use sputnik_editor::core::{Action, Motion};
///
/// assert_eq!(Action::from(Motion::Left), Action::Move(Motion::Left));
/// ```
impl From<Motion> for Action {
    fn from(motion: Motion) -> Self {
        Action::Move(motion)
    }
}

/// Shorthand for [`Action::Edit`].
///
/// ```
/// use sputnik_editor::core::{Action, Edit};
///
/// assert_eq!(Action::from(Edit::Enter), Action::Edit(Edit::Enter));
/// ```
impl From<Edit> for Action {
    fn from(edit: Edit) -> Self {
        Action::Edit(edit)
    }
}
