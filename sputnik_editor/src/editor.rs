//! The handle a host holds on to: a document, plus where it is being
//! looked at from.

use std::cell::Cell;

use ropey::Rope;

use crate::core::{Action, Document, Selection, Text};
use crate::visual::VisualLayout;
use crate::widget::TextEditor;

/// Everything about the view that a document has no business knowing, and
/// that only rendering can measure.
///
/// The widget writes this during layout; [`Editor::perform`] reads it back
/// to resolve motions that depend on wrapping. That is the whole reason it
/// is shared: it is measurement, not model.
///
/// # Examples
///
/// ```
/// use sputnik_editor::Viewport;
///
/// let viewport = Viewport {
///     visible_rows: 30,
///     ..Viewport::default()
/// };
/// assert_eq!(viewport.scroll, (0, 0.0));
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// Topmost visible line, plus how many pixels of it sit above the
    /// viewport's top edge, always less than that line's rendered height.
    ///
    /// Anchoring to a line rather than to an absolute pixel offset means
    /// the height of a row nobody has shaped never has to be guessed: only
    /// rows next to what is already visible are measured, while the pixel
    /// offset still gives smooth scrolling within them.
    pub scroll: (usize, f32),
    /// The width text wraps at: the widget's bounds less its gutter.
    pub wrap_width: f32,
    /// Font size in pixels.
    pub font_size: f32,
    /// Height of one unwrapped row in pixels.
    pub line_height: f32,
    /// How many rows fit on screen, which is what a page-up or page-down
    /// travels.
    pub visible_rows: usize,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            scroll: (0, 0.0),
            wrap_width: 800.0,
            font_size: 16.0,
            line_height: 20.0,
            visible_rows: 1,
        }
    }
}

/// A document being edited through a view.
///
/// Hosts own one of these, change it with [`Editor::perform`], and render
/// it with [`Editor::view`]. It is not generic over a message type: what a
/// click means is decided where the widget is built, not here.
///
/// # Examples
///
/// ```
/// use sputnik_editor::{Action, Edit, Editor, Motion};
///
/// let mut editor = Editor::<String>::from_str("hello world");
///
/// editor.perform(Action::Move(Motion::To(5)));
/// editor.perform(Action::Select(Motion::DocumentEnd));
/// assert_eq!(editor.document().selected_text(), " world");
///
/// editor.perform(Action::Edit(Edit::Paste("!".into())));
/// assert_eq!(editor.text(), "hello!");
/// ```
#[derive(Debug)]
pub struct Editor<T: Text = Rope> {
    document: Document<T>,
    viewport: Cell<Viewport>,
}

impl<T: Text> Editor<T> {
    /// Puts a view in front of `document`.
    pub fn new(document: Document<T>) -> Self {
        Self {
            document,
            viewport: Cell::new(Viewport::default()),
        }
    }

    /// Builds an editor holding `text`.
    // Mirrors the constructor on the storage itself; `FromStr` would only
    // add `.parse()`, which reads worse and implies a fallibility this has
    // none of.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(text: &str) -> Self {
        Self::new(Document::from_str(text))
    }

    /// Puts a different document in front of the same view — opening a
    /// file — keeping everything the widget has measured and only
    /// returning the scroll to the top.
    ///
    /// The measurements describe the widget, which has not changed; only
    /// the text has. Discarding them would leave the next keystroke
    /// resolving `Up`, `Down` and `PageDown` against defaults until a
    /// layout pass happened to run first.
    ///
    /// ```
    /// use sputnik_editor::{Document, Editor};
    ///
    /// let mut editor = Editor::<String>::from_str("old");
    /// editor.open(Document::from_str("new"));
    /// assert_eq!(editor.text(), "new");
    /// assert_eq!(editor.viewport().scroll, (0, 0.0));
    /// ```
    pub fn open(&mut self, document: Document<T>) {
        self.document = document;
        self.viewport.set(Viewport {
            scroll: (0, 0.0),
            ..self.viewport.get()
        });
    }

    /// The document being edited.
    pub fn document(&self) -> &Document<T> {
        &self.document
    }

    /// The document being edited, mutably.
    pub fn document_mut(&mut self) -> &mut Document<T> {
        &mut self.document
    }

    /// The stored text.
    pub fn text_storage(&self) -> &T {
        self.document.text_storage()
    }

    /// The caret position.
    pub fn cursor(&self) -> usize {
        self.document.cursor()
    }

    /// The current selection.
    pub fn selection(&self) -> Selection {
        self.document.selection()
    }

    /// The whole document as an owned string.
    pub fn text(&self) -> String {
        self.document.text()
    }

    /// What the widget measured during its last layout pass.
    pub fn viewport(&self) -> Viewport {
        self.viewport.get()
    }

    /// Applies an action, resolving any wrap-dependent motion against the
    /// view as it was last laid out.
    pub fn perform(&mut self, action: Action) {
        let layout = VisualLayout::new(self.viewport.get());
        self.document.perform(action, &layout);
    }

    /// The widget for this editor.
    ///
    /// Wire [`TextEditor::on_interaction`] to give it a mouse; leave it off
    /// and the mouse does nothing at all. Stack [`TextEditor::layer`] to
    /// change what gets drawn.
    pub fn view<Message>(&self) -> TextEditor<'_, Message, T, iced::Theme> {
        TextEditor::new(&self.document, &self.viewport)
    }
}

impl<T: Text + Default> Default for Editor<T> {
    fn default() -> Self {
        Self::new(Document::default())
    }
}
