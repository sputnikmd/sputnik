//! Text plus a selection, and the rules connecting them.

use ropey::Rope;

use crate::core::{Action, Edit, Layout, Motion, Selection, Text};

/// An editable document: some [`Text`] and the [`Selection`] into it.
///
/// This is the whole editor model. It knows nothing of pixels, keys or
/// widgets: [`Document::perform`] is the only way in, and the only thing
/// it borrows from outside is a [`Layout`], for the motions that genuinely
/// cannot be answered from text alone.
///
/// Storage is a type parameter, so the same model drives a rope-backed
/// editor, a plain `String` in a test, or anything else implementing
/// [`Text`].
///
/// # Examples
///
/// ```
/// use sputnik_editor::core::{Action, Document, Edit, LogicalLayout, Motion};
///
/// let layout = LogicalLayout::default();
/// let mut document = Document::<String>::from_str("hello world");
///
/// document.perform(Action::Move(Motion::To(6)), &layout);
/// document.perform(Action::Select(Motion::DocumentEnd), &layout);
/// assert_eq!(document.selected_text(), "world");
///
/// document.perform(Action::Edit(Edit::Backspace), &layout);
/// assert_eq!(document.text(), "hello ");
/// ```
#[derive(Debug, Clone)]
pub struct Document<T: Text = Rope> {
    text: T,
    selection: Selection,
    tab_width: usize,
}

impl<T: Text> Document<T> {
    /// Wraps existing storage, with the caret at the start.
    pub fn new(text: T) -> Self {
        Self {
            text,
            selection: Selection::default(),
            tab_width: 4,
        }
    }

    /// Builds a document holding `text`.
    ///
    /// ```
    /// use sputnik_editor::core::Document;
    ///
    /// let document = Document::<String>::from_str("hi");
    /// assert_eq!(document.text(), "hi");
    /// ```
    // Mirrors the constructor on the storage itself; `FromStr` would only
    // add `.parse()`, which reads worse and implies a fallibility this has
    // none of.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(text: &str) -> Self {
        Self::new(T::from_str(text))
    }

    /// The stored text.
    pub fn text_storage(&self) -> &T {
        &self.text
    }

    /// The current selection.
    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// The caret: the moving end of the selection.
    pub fn cursor(&self) -> usize {
        self.selection.head
    }

    /// How many spaces one [`Edit::Tab`] inserts.
    pub fn tab_width(&self) -> usize {
        self.tab_width
    }

    /// Sets how many spaces one [`Edit::Tab`] inserts. Values below one
    /// are raised to one.
    pub fn set_tab_width(&mut self, tab_width: usize) {
        self.tab_width = tab_width.max(1);
    }

    /// Moves the selection wholesale, snapping both ends into the text so
    /// that positions of unknown provenance are safe to pass.
    ///
    /// ```
    /// use sputnik_editor::core::{Document, Selection};
    ///
    /// let mut document = Document::<String>::from_str("hi");
    /// document.set_selection(Selection::new(0, 900));
    /// assert_eq!(document.selection(), Selection::new(0, 2));
    /// ```
    pub fn set_selection(&mut self, selection: Selection) {
        self.selection = selection.clamped(&self.text);
    }

    /// The whole document as an owned string.
    pub fn text(&self) -> String {
        self.text.substring(0..self.text.len())
    }

    /// The selected text, empty when the selection is only a caret.
    pub fn selected_text(&self) -> String {
        self.text.substring(self.selection.range())
    }

    /// Applies one [`Action`]: the single entry point for every change.
    pub fn perform(&mut self, action: Action, layout: &impl Layout) {
        match action {
            Action::Move(motion) => {
                // Collapsing a selection sideways is not the same as moving
                // its head: the caret belongs on the edge it is heading
                // for, not one step past it.
                let target = match (self.selection.is_empty(), motion) {
                    (false, Motion::Left) => self.selection.start(),
                    (false, Motion::Right) => self.selection.end(),
                    _ => self.target(motion, layout),
                };
                self.selection = Selection::caret(target);
            }
            Action::Select(motion) => {
                self.selection.head = self.target(motion, layout);
            }
            Action::SelectAll => {
                self.selection = Selection::new(0, self.text.len());
            }
            Action::Edit(edit) => self.edit(edit),
        }
    }

    /// Where `motion` would put the caret.
    fn target(&self, motion: Motion, layout: &impl Layout) -> usize {
        let at = self.selection.head;
        let page = isize::try_from(layout.page_rows().max(1)).unwrap_or(isize::MAX);

        match motion {
            Motion::Left => self.text.prev(at),
            Motion::Right => self.text.next(at),
            Motion::WordLeft => self.text.prev_word(at),
            Motion::WordRight => self.text.next_word(at),
            Motion::RowStart => layout.row_start(&self.text, at),
            Motion::RowEnd => layout.row_end(&self.text, at),
            Motion::Up => layout.vertical(&self.text, at, -1),
            Motion::Down => layout.vertical(&self.text, at, 1),
            Motion::PageUp => layout.vertical(&self.text, at, -page),
            Motion::PageDown => layout.vertical(&self.text, at, page),
            Motion::DocumentStart => 0,
            Motion::DocumentEnd => self.text.len(),
            Motion::To(at) => self.text.clamp(at),
        }
    }

    fn edit(&mut self, edit: Edit) {
        match edit {
            Edit::Insert(character) => {
                self.replace_selection(character.encode_utf8(&mut [0; 4]));
            }
            Edit::Paste(text) => self.replace_selection(&text),
            Edit::Enter => self.replace_selection("\n"),
            Edit::Tab => self.replace_selection(&" ".repeat(self.tab_width)),
            Edit::Backspace => {
                if self.selection.is_empty() {
                    let head = self.selection.head;
                    let from = self.text.prev(head);
                    self.text.remove(from..head);
                    self.selection = Selection::caret(from);
                } else {
                    self.delete_selection();
                }
            }
            Edit::Delete => {
                if self.selection.is_empty() {
                    let head = self.selection.head;
                    let to = self.text.next(head);
                    self.text.remove(head..to);
                    self.selection = Selection::caret(head);
                } else {
                    self.delete_selection();
                }
            }
        }
    }

    /// Removes the selected text and leaves a caret where it was, returning
    /// that position. Collapsing an already-empty selection is a no-op, so
    /// every edit can call this unconditionally.
    fn delete_selection(&mut self) -> usize {
        let range = self.selection.clamped(&self.text).range();
        self.text.remove(range.clone());
        self.selection = Selection::caret(range.start);
        range.start
    }

    /// Writes `text` over the selection, leaving the caret after it.
    fn replace_selection(&mut self, text: &str) {
        let at = self.delete_selection();
        self.text.insert(at, text);
        self.selection = Selection::caret(at + text.len());
    }
}

impl<T: Text + Default> Default for Document<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::LogicalLayout;

    const LAYOUT: LogicalLayout = LogicalLayout { page_rows: 3 };

    fn document(text: &str) -> Document<String> {
        Document::from_str(text)
    }

    fn perform(document: &mut Document<String>, actions: impl IntoIterator<Item = Action>) {
        for action in actions {
            document.perform(action, &LAYOUT);
        }
    }

    fn type_text(document: &mut Document<String>, text: &str) {
        perform(
            document,
            text.chars()
                .map(|character| Action::Edit(Edit::Insert(character))),
        );
    }

    #[test]
    fn typing_and_backspace_without_a_selection() {
        let mut document = document("");
        type_text(&mut document, "ab");
        assert_eq!(document.text(), "ab");
        assert_eq!(document.cursor(), 2);

        perform(&mut document, [Action::Edit(Edit::Backspace)]);
        assert_eq!(document.text(), "a");
        assert_eq!(document.cursor(), 1);

        // Backspace at the very start is a no-op rather than an underflow.
        perform(
            &mut document,
            std::iter::repeat_n(Action::Edit(Edit::Backspace), 5),
        );
        assert_eq!(document.text(), "");
        assert_eq!(document.cursor(), 0);
    }

    #[test]
    fn backspace_deletes_the_selection_and_leaves_a_caret_in_its_place() {
        let mut document = document("hello world");
        document.set_selection(Selection::new(6, 11));

        perform(&mut document, [Action::Edit(Edit::Backspace)]);

        assert_eq!(document.text(), "hello ");
        assert_eq!(document.selection(), Selection::caret(6));
    }

    #[test]
    fn delete_forward_removes_the_selection_and_nothing_more() {
        let mut document = document("hello world");
        document.set_selection(Selection::new(0, 5));

        perform(&mut document, [Action::Edit(Edit::Delete)]);

        assert_eq!(document.text(), " world");
        assert_eq!(document.selection(), Selection::caret(0));
    }

    #[test]
    fn a_backwards_selection_deletes_the_same_text() {
        let mut document = document("hello world");
        document.set_selection(Selection::new(11, 6));

        perform(&mut document, [Action::Edit(Edit::Backspace)]);

        assert_eq!(document.text(), "hello ");
        assert_eq!(document.selection(), Selection::caret(6));
    }

    #[test]
    fn typing_replaces_the_selection_exactly_once() {
        let mut document = document("hello world");
        document.set_selection(Selection::new(6, 11));

        // A host types character by character: the first consumes the
        // selection and the rest simply follow the caret.
        type_text(&mut document, "ab");

        assert_eq!(document.text(), "hello ab");
        assert_eq!(document.cursor(), 8);
        assert!(document.selection().is_empty());
    }

    #[test]
    fn enter_tab_and_paste_replace_the_selection_too() {
        for (edit, expected) in [
            (Edit::Enter, "hello \n"),
            (Edit::Tab, "hello     "),
            (Edit::Paste("!".into()), "hello !"),
        ] {
            let mut document = document("hello world");
            document.set_selection(Selection::new(6, 11));
            perform(&mut document, [Action::Edit(edit)]);
            assert_eq!(document.text(), expected);
            assert!(document.selection().is_empty());
        }
    }

    #[test]
    fn deleting_multibyte_text_keeps_positions_valid() {
        let mut document = document("héllo wörld");
        perform(
            &mut document,
            [Action::SelectAll, Action::Edit(Edit::Backspace)],
        );

        assert_eq!(document.text(), "");
        assert_eq!(document.selection(), Selection::caret(0));
    }

    #[test]
    fn plain_motion_collapses_a_selection_onto_the_edge_it_heads_for() {
        let mut document = document("hello world");

        document.set_selection(Selection::new(2, 8));
        perform(&mut document, [Action::Move(Motion::Left)]);
        assert_eq!(document.selection(), Selection::caret(2));

        document.set_selection(Selection::new(2, 8));
        perform(&mut document, [Action::Move(Motion::Right)]);
        assert_eq!(document.selection(), Selection::caret(8));
    }

    #[test]
    fn select_extends_from_a_fixed_anchor() {
        let mut document = document("hello world");
        perform(
            &mut document,
            [
                Action::Move(Motion::To(5)),
                Action::Select(Motion::Right),
                Action::Select(Motion::Right),
            ],
        );
        assert_eq!(document.selection(), Selection::new(5, 7));
        assert_eq!(document.selected_text(), " w");

        // Reversing past the anchor leaves it put and flips direction.
        perform(&mut document, [Action::Select(Motion::To(1))]);
        assert_eq!(document.selection(), Selection::new(5, 1));
        assert_eq!(document.selected_text(), "ello");
    }

    #[test]
    fn select_all_then_typing_replaces_the_document() {
        let mut document = document("throw this away");
        perform(&mut document, [Action::SelectAll]);
        assert_eq!(document.selection(), Selection::new(0, 15));

        type_text(&mut document, "new");
        assert_eq!(document.text(), "new");
    }

    #[test]
    fn a_position_far_past_the_end_snaps_into_the_document() {
        let mut document = document("short");
        perform(&mut document, [Action::Move(Motion::To(9_999))]);
        assert_eq!(document.cursor(), 5);
    }

    #[test]
    fn paging_travels_the_layouts_page_size() {
        let mut document = document("0\n1\n2\n3\n4\n5\n6");
        perform(&mut document, [Action::Move(Motion::PageDown)]);
        assert_eq!(document.text_storage().line_of(document.cursor()), 3);
    }

    /// The model is storage-agnostic, so the same script must produce the
    /// same result whichever [`Text`] backs it.
    #[test]
    fn every_storage_produces_the_same_result() {
        fn script<T: Text>() -> (String, Selection) {
            let mut document = Document::<T>::from_str("the quick brown fox");
            for action in [
                Action::Move(Motion::To(4)),
                Action::Select(Motion::WordRight),
                Action::Edit(Edit::Paste("slow".into())),
                Action::Select(Motion::DocumentStart),
            ] {
                document.perform(action, &LAYOUT);
            }
            (document.text(), document.selection())
        }

        assert_eq!(script::<Rope>(), script::<String>());
        assert_eq!(script::<String>().0, "the slow brown fox");
    }
}
