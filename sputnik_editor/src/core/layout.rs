//! The one thing a document cannot work out alone: how its text is
//! arranged on screen.

use crate::core::Text;

/// Resolves the motions that depend on rendering rather than on text.
///
/// With soft wrap, "one row up" is not "one line up", and only whoever
/// shaped the glyphs knows where a row begins or ends. Rather than let a
/// document reach for a font system — which would weld it to one
/// front-end — it asks through this trait.
///
/// A terminal front-end supplies [`LogicalLayout`] if it does not wrap, or
/// its own implementation if it does.
///
/// # Examples
///
/// ```
/// use sputnik_editor::core::{Layout, LogicalLayout, Text};
///
/// let text = String::from("first\nsecond\nthird");
/// let layout = LogicalLayout { page_rows: 2 };
///
/// // Column 3 of the first line, one row down.
/// assert_eq!(layout.vertical(&text, 3, 1), 9);
/// // Two rows down is a page here.
/// assert_eq!(layout.page_rows(), 2);
/// ```
pub trait Layout {
    /// The position `rows` rendered rows away from `at` — negative up,
    /// positive down — keeping the horizontal position as closely as the
    /// target row allows.
    ///
    /// Clamps at the ends of the document rather than failing: travelling
    /// up from the first row lands on position 0, and down from the last
    /// lands on the end, so a held-down arrow key settles instead of
    /// stalling mid-line.
    fn vertical<T: Text + ?Sized>(&self, text: &T, at: usize, rows: isize) -> usize;

    /// How many rows a page-up or page-down travels, normally the number
    /// of rows on screen.
    fn page_rows(&self) -> usize {
        1
    }

    /// First byte of the rendered row containing `at`.
    ///
    /// Defaults to the start of the logical line, which is exactly right
    /// for any layout that does not soft-wrap.
    fn row_start<T: Text + ?Sized>(&self, text: &T, at: usize) -> usize {
        text.line_start(text.line_of(at))
    }

    /// Byte just past the last character of the rendered row containing
    /// `at`, defaulting to the end of the logical line as
    /// [`Layout::row_start`] does.
    fn row_end<T: Text + ?Sized>(&self, text: &T, at: usize) -> usize {
        text.line_end(text.line_of(at))
    }
}

/// A [`Layout`] that ignores wrapping: one row is one line, and the
/// horizontal position is a character offset into it.
///
/// Enough on its own for a front-end without soft wrap, and the natural
/// choice in tests, where shaping real glyphs would be beside the point.
///
/// # Examples
///
/// ```
/// use sputnik_editor::core::{Layout, LogicalLayout, Text};
///
/// let text = String::from("abcdef\nxy\nghijkl");
/// let layout = LogicalLayout::default();
///
/// // Column 4 has nowhere to land on a two-character line, so it clamps.
/// assert_eq!(layout.vertical(&text, 4, 1), 9);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogicalLayout {
    /// How many rows a page-up or page-down travels.
    pub page_rows: usize,
}

impl Default for LogicalLayout {
    fn default() -> Self {
        Self { page_rows: 1 }
    }
}

impl Layout for LogicalLayout {
    fn vertical<T: Text + ?Sized>(&self, text: &T, at: usize, rows: isize) -> usize {
        let at = text.clamp(at);
        let line = text.line_of(at);

        let target = match line.checked_add_signed(rows) {
            None => return 0,
            Some(target) if target >= text.line_count() => return text.len(),
            Some(target) => target,
        };

        // Counted in characters rather than bytes, so a line of multi-byte
        // text lines up with an ASCII one above it.
        let column = text
            .chunks(text.line_start(line)..at)
            .flat_map(str::chars)
            .count();

        let start = text.line_start(target);
        let mut position = start;
        for character in text
            .chunks(start..text.line_end(target))
            .flat_map(str::chars)
            .take(column)
        {
            position += character.len_utf8();
        }
        position
    }

    fn page_rows(&self) -> usize {
        self.page_rows.max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_column_is_kept_and_clamped_to_short_lines() {
        let text = String::from("abcdef\nxy\nghijkl");
        let layout = LogicalLayout::default();

        assert_eq!(layout.vertical(&text, 4, 1), text.line_start(1) + 2);
        assert_eq!(
            layout.vertical(&text, text.line_start(1) + 2, 1),
            text.line_start(2) + 2
        );
    }

    #[test]
    fn columns_are_characters_rather_than_bytes() {
        // Each "é" occupies two bytes, so column 2 of the first line is
        // byte 4 and must still line up with column 2 below it.
        let text = String::from("ééxx\nabcd");
        let layout = LogicalLayout::default();
        assert_eq!(layout.vertical(&text, 4, 1), text.line_start(1) + 2);
    }

    #[test]
    fn travelling_past_either_end_lands_on_the_documents_bounds() {
        let text = String::from("one\ntwo\nthree");
        let layout = LogicalLayout::default();
        assert_eq!(layout.vertical(&text, 2, -1), 0);
        assert_eq!(layout.vertical(&text, 2, -50), 0);
        assert_eq!(layout.vertical(&text, 9, 50), Text::len(&text));
    }

    #[test]
    fn rows_default_to_logical_line_bounds() {
        let text = String::from("first\nsecond");
        let layout = LogicalLayout::default();
        assert_eq!(layout.row_start(&text, 8), 6);
        assert_eq!(layout.row_end(&text, 8), 12);
    }
}
