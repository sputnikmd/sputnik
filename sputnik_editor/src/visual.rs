//! The wrap-aware [`Layout`] the iced front-end hands to a document.

use std::borrow::Cow;

use crate::core::{Layout, Text};
use crate::editor::Viewport;

/// Answers "which row is up from here?" by shaping the text around the
/// caret with `cosmic-text`, at the width the widget wraps to.
///
/// Only the lines immediately around the caret are shaped, never the
/// document, so a keystroke costs the same in a huge file as in a small
/// one. It deliberately does not reuse the widget's shaped rows: those
/// exist only while the caret is on screen, and a motion has to work when
/// it is not.
///
/// # Examples
///
/// ```
/// use sputnik_editor::core::Layout;
/// use sputnik_editor::{Viewport, VisualLayout};
///
/// let layout = VisualLayout::new(Viewport {
///     visible_rows: 24,
///     ..Viewport::default()
/// });
/// assert_eq!(layout.page_rows(), 24);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct VisualLayout {
    viewport: Viewport,
}

impl VisualLayout {
    /// Resolves motions against the view described by `viewport`.
    pub fn new(viewport: Viewport) -> Self {
        Self { viewport }
    }
}

impl Layout for VisualLayout {
    fn vertical<T: Text + ?Sized>(&self, text: &T, at: usize, rows: isize) -> usize {
        let direction = rows.signum();
        if direction == 0 {
            return text.clamp(at);
        }

        // Paging repeats a single step rather than shaping one wide window.
        // A page is bounded by the viewport, so this is a few dozen
        // shapings of two lines each at worst, and it keeps one code path
        // that the far more common single-row case exercises constantly.
        let mut at = text.clamp(at);
        for _ in 0..rows.unsigned_abs() {
            let next = step(&self.viewport, text, at, direction);
            if next == at {
                break;
            }
            at = next;
        }
        at
    }

    fn page_rows(&self) -> usize {
        self.viewport.visible_rows.max(1)
    }
}

/// One rendered row up (`direction < 0`) or down (`direction > 0`).
///
/// Returns `at` unchanged when there is nowhere left to go, which is how
/// [`VisualLayout::vertical`] knows to stop paging early.
fn step<T: Text + ?Sized>(viewport: &Viewport, text: &T, at: usize, direction: isize) -> usize {
    let line = text.line_of(at);
    let line_count = text.line_count();

    // Shape just enough context to see the neighbouring row: the line
    // before the caret's when going up, the one after it when going down.
    // A wrapped line supplies its own extra rows, so this always suffices.
    let (first, last) = if direction < 0 {
        (line.saturating_sub(1), line + 1)
    } else {
        (line, (line + 2).min(line_count))
    };

    let origin = text.line_start(first);
    let end = if last < line_count {
        text.line_start(last)
    } else {
        text.len()
    };
    if end <= origin {
        return at;
    }
    // Borrowed outright whenever the neighbourhood lies inside a single
    // storage chunk, which covers everything but the longest lines.
    let mut chunks = text.chunks(origin..end);
    let source: Cow<'_, str> = match (chunks.next(), chunks.next()) {
        (Some(single), None) => Cow::Borrowed(single),
        (Some(first), Some(second)) => {
            let mut joined = String::with_capacity(end - origin);
            joined.push_str(first);
            joined.push_str(second);
            joined.extend(chunks);
            Cow::Owned(joined)
        }
        _ => Cow::Borrowed(""),
    };

    let font_system = iced::advanced::graphics::text::font_system();
    let mut font_system = font_system.write().expect("acquire font system");

    let metrics = cosmic_text::Metrics::new(viewport.font_size, viewport.line_height);
    let mut shaped = cosmic_text::Buffer::new(font_system.raw(), metrics);
    shaped.set_size(Some(viewport.wrap_width), None);
    shaped.set_text(
        &source,
        &cosmic_text::Attrs::new(),
        cosmic_text::Shaping::Advanced,
        None,
    );
    shaped.shape_until_scroll(font_system.raw(), false);
    drop(font_system);

    // Where each shaped line starts, relative to `origin`. Taken from the
    // text rather than by summing shaped line lengths, so that a `\r\n`
    // break — two bytes, not one — cannot shift every following offset.
    let line_starts: Vec<usize> = (0..shaped.lines.len())
        .map(|index| {
            let line = first + index;
            if line < line_count {
                text.line_start(line) - origin
            } else {
                end - origin
            }
        })
        .collect();

    let runs: Vec<_> = shaped.layout_runs().collect();
    let mut rows: Vec<ShapedRow> = runs
        .iter()
        .enumerate()
        .map(|(index, run)| {
            let base = line_starts.get(run.line_i).copied().unwrap_or(0);
            let (start, end) = match run.glyphs {
                [] => (0, 0),
                glyphs => (
                    glyphs.iter().map(|glyph| glyph.start).min().unwrap_or(0),
                    glyphs.iter().map(|glyph| glyph.end).max().unwrap_or(0),
                ),
            };
            ShapedRow {
                start: base + start,
                end: base + end,
                run: index,
                line: run.line_i,
            }
        })
        .collect();
    rows.sort_by_key(|row| row.start);

    if rows.is_empty() {
        return at;
    }

    let local = at - origin;
    let current = rows
        .iter()
        .position(|row| local >= row.start && local <= row.end)
        .or_else(|| rows.iter().rposition(|row| row.start <= local))
        .unwrap_or(0);

    let target = (current as isize + direction).clamp(0, rows.len() as isize - 1) as usize;
    if target == current {
        // Already on the first or last row: travel to the document's edge
        // rather than refuse to move, so repeated presses settle there.
        return if direction < 0 { 0 } else { text.len() };
    }

    // The caret's horizontal position on its current row, to aim for.
    let row = &rows[current];
    let base = line_starts.get(row.line).copied().unwrap_or(0);
    let cursor = cosmic_text::Cursor::new(row.line, local.saturating_sub(base));
    let goal = runs[row.run].cursor_position(&cursor).unwrap_or(0.0);

    let row = &rows[target];
    let base = line_starts.get(row.line).copied().unwrap_or(0);
    let mut best = row.start;
    let mut best_distance = f32::MAX;
    for glyph in runs[row.run].glyphs {
        // Glyph offsets are relative to the logical line, not to this
        // visual row. Where wrapping splits one line across several rows,
        // `row.start` already carries the row's own offset, so the logical
        // line's base is the only origin that counts each contribution
        // exactly once.
        let start = base + glyph.start;
        let end = (base + glyph.end).min(row.end);
        for (x, offset) in [(glyph.x, start), (glyph.x + glyph.w, end)] {
            let distance = (x - goal).abs();
            if distance < best_distance {
                best_distance = distance;
                best = offset;
            }
        }
    }

    text.clamp(origin + best)
}

/// One visual row of the shaped neighbourhood, with its byte range
/// relative to the shaped text.
struct ShapedRow {
    start: usize,
    end: usize,
    /// Index into the collected layout runs.
    run: usize,
    /// The logical line this row belongs to.
    line: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(wrap_width: f32) -> VisualLayout {
        VisualLayout::new(Viewport {
            wrap_width,
            font_size: 24.0,
            line_height: 30.0,
            visible_rows: 6,
            ..Viewport::default()
        })
    }

    /// Walks the caret to the top of `text`, collecting every position.
    fn walk_up(layout: &VisualLayout, text: &str) -> Vec<usize> {
        let text = text.to_owned();
        let mut at = text.len();
        let mut visited = Vec::new();
        for _ in 0..text.len() + 10 {
            at = layout.vertical(&text, at, -1);
            visited.push(at);
        }
        visited
    }

    fn assert_monotonic(visited: &[usize]) {
        for pair in visited.windows(2) {
            assert!(
                pair[1] <= pair[0],
                "travelling up must never move forward, but went {} -> {}",
                pair[0],
                pair[1]
            );
        }
    }

    /// An empty line has no glyphs to anchor against, so its row has to be
    /// recognised from its position alone.
    #[test]
    fn moving_up_through_empty_lines_only_ever_moves_up() {
        let visited = walk_up(&layout(800.0), "zero\none\n\nthree\n\nfive\nsix\n");
        assert_monotonic(&visited);
        assert_eq!(visited.last(), Some(&0), "it should settle at the start");
    }

    /// A line narrow enough to wrap contributes several rows of its own,
    /// which the row bookkeeping has to keep in order.
    #[test]
    fn moving_up_out_of_a_wrapped_line_only_ever_moves_up() {
        let visited = walk_up(
            &layout(120.0),
            "The quick brown fox jumps over the lazy dog\n\nEnd\n",
        );
        assert_monotonic(&visited);
    }

    #[test]
    fn a_wrapped_line_is_several_rows_tall_to_travel_through() {
        let text = "wrap ".repeat(20);
        let layout = layout(120.0);
        assert!(
            layout.vertical(&text, Text::len(&text), -1) > 0,
            "one row up inside a wrapped line must stay inside it"
        );
    }

    #[test]
    fn paging_travels_a_viewports_worth_of_rows() {
        let text: String = (1..=20).map(|index| format!("line {index}\n")).collect();
        let layout = layout(800.0);
        assert_eq!(layout.page_rows(), 6);

        let paged = layout.vertical(&text, 0, layout.page_rows() as isize);
        assert_eq!(text.line_of(paged), 6);
    }
}
