//! Where the caret is, and what is selected.

use std::ops::Range;

use crate::core::Text;

/// A caret and a selection in one value.
///
/// `anchor` is the end that stays put; `head` is the end that moves, and
/// is also the caret. When the two coincide nothing is selected — which is
/// why there is no `Option` here and no separate cursor field. Collapsing
/// the two ideas is what makes shift-extend, drag-to-select and
/// replace-on-typing fall out of the same few lines instead of needing two
/// fields kept in step with each other.
///
/// `anchor` may be greater than `head`: a selection made backwards keeps
/// its direction, so extending it further moves the end the user is
/// actually dragging. Reach for [`Selection::range`] whenever ascending
/// order is what matters.
///
/// # Examples
///
/// ```
/// use sputnik_editor::core::Selection;
///
/// let backwards = Selection::new(8, 2);
/// assert_eq!(backwards.range(), 2..8);
/// assert_eq!(backwards.head, 2);
///
/// assert!(Selection::caret(4).is_empty());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    /// The end that stays put while the selection is extended.
    pub anchor: usize,
    /// The end that moves, and the caret.
    pub head: usize,
}

impl Selection {
    /// A collapsed selection — just a caret — at `at`.
    pub fn caret(at: usize) -> Self {
        Self {
            anchor: at,
            head: at,
        }
    }

    /// A selection running from `anchor` to `head`, in either direction.
    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    /// Whether nothing is selected and this is only a caret.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// The selected bytes in ascending order, whichever way round the
    /// selection was made.
    pub fn range(&self) -> Range<usize> {
        self.start()..self.end()
    }

    /// The lower of the two ends.
    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    /// The higher of the two ends.
    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    /// Snaps both ends into `text`, so a selection that outlived the text
    /// it pointed at degrades into a valid nearby one.
    ///
    /// ```
    /// use sputnik_editor::core::Selection;
    ///
    /// let text = String::from("hi");
    /// assert_eq!(Selection::new(2, 900).clamped(&text), Selection::new(2, 2));
    /// ```
    pub fn clamped(self, text: &(impl Text + ?Sized)) -> Self {
        Self {
            anchor: text.clamp(self.anchor),
            head: text.clamp(self.head),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_is_preserved_while_the_range_stays_ascending() {
        let backwards = Selection::new(8, 2);
        assert_eq!(backwards.range(), 2..8);
        assert_eq!((backwards.anchor, backwards.head), (8, 2));
        assert!(!backwards.is_empty());
    }

    #[test]
    fn a_caret_is_an_empty_selection() {
        let caret = Selection::caret(4);
        assert!(caret.is_empty());
        assert_eq!(caret.range(), 4..4);
    }

    #[test]
    fn clamping_survives_text_that_shrank_underneath() {
        let text = String::from("hi");
        assert_eq!(Selection::new(2, 900).clamped(&text), Selection::new(2, 2));
    }
}
