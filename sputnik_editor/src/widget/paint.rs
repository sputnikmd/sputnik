//! Drawing the caret and the selection, and turning a click back into a
//! position.
//!
//! All three read the `cosmic-text` buffers that row shaping has already
//! produced, and shape nothing themselves. Every position crosses between
//! document and screen coordinates through a row's
//! [`Mapping`](crate::core::Mapping), which is what keeps them right once
//! layers have hidden, added or substituted text.

use std::ops::Range;

use iced::advanced::renderer::{Quad, Renderer};
use iced::advanced::text::Paragraph as _;
use iced::{Background, Color, Point, Rectangle};

use crate::widget::row::ShapedRow;

const SELECTION: Color = Color::from_rgba(0.2, 0.5, 1.0, 0.35);

/// Highlights the part of `selection` falling on each visible row.
pub fn selection<R: Renderer>(
    renderer: &mut R,
    rows: &[ShapedRow],
    origin: Point,
    selection: Range<usize>,
) {
    if selection.start >= selection.end {
        return;
    }

    for row in rows {
        // Compared against the row's line rather than the bytes it draws,
        // so a selection reaching past the end of a line still highlights
        // through the line break.
        if selection.end <= row.source.start || selection.start > row.source.end {
            continue;
        }

        // Both ends go through the mapping, so a selection spanning text a
        // layer hid stays one contiguous highlight instead of collapsing.
        let start = row.mapping.to_drawn(selection.start.max(row.source.start));
        let end = row.mapping.to_drawn(selection.end.min(row.source.end));
        if start >= end {
            continue;
        }

        let hint = row.paragraph.hint_factor().unwrap_or(1.0);
        let buffer = row.paragraph.buffer();
        let length = buffer
            .lines
            .first()
            .map(|line| line.text().len())
            .unwrap_or(0);
        let from = cosmic_text::Cursor::new(0, start.min(length));
        let to = cosmic_text::Cursor::new(0, end.min(length));

        for run in buffer.layout_runs() {
            for (x, width) in run.highlight(from, to) {
                renderer.fill_quad(
                    Quad {
                        bounds: Rectangle {
                            x: origin.x + x / hint,
                            y: origin.y + row.y + run.line_top / hint,
                            width: width / hint,
                            height: (run.line_height + 1.0) / hint,
                        },
                        border: iced::Border::default(),
                        shadow: iced::Shadow::default(),
                        snap: true,
                    },
                    Background::Color(SELECTION),
                );
            }
        }
    }
}

/// Draws the caret, or nothing when it has scrolled out of the window.
///
/// Drawing nothing beats clamping to the nearest edge, which would show a
/// caret where there is none.
pub fn cursor<R: Renderer>(
    renderer: &mut R,
    rows: &[ShapedRow],
    origin: Point,
    cursor: usize,
    color: Color,
    width: f32,
) {
    let (Some(first), Some(last)) = (rows.first(), rows.last()) else {
        return;
    };
    if cursor < first.source.start || cursor > last.source.end {
        return;
    }

    // The last row starting at or before the caret.
    let index = match rows.binary_search_by_key(&cursor, |row| row.source.start) {
        Ok(index) => index,
        Err(0) => 0,
        Err(index) => index - 1,
    };
    let Some(row) = rows.get(index) else {
        return;
    };

    let hint = row.paragraph.hint_factor().unwrap_or(1.0);
    let Some((x, y, height)) = caret_in(row, row.mapping.to_drawn(cursor), hint) else {
        return;
    };

    renderer.fill_quad(
        Quad {
            bounds: Rectangle {
                x: origin.x + x,
                y: origin.y + row.y + y,
                width,
                height,
            },
            border: iced::Border::default(),
            shadow: iced::Shadow::default(),
            snap: true,
        },
        Background::Color(color),
    );
}

/// The caret's position within one row, which is always logical line 0 of
/// that row's own buffer even when wrapping splits it across several runs.
fn caret_in(row: &ShapedRow, offset: usize, hint: f32) -> Option<(f32, f32, f32)> {
    let buffer = row.paragraph.buffer();
    let length = buffer
        .lines
        .first()
        .map(|line| line.text().len())
        .unwrap_or(0);
    let cursor = cosmic_text::Cursor::new(0, offset.min(length));

    for run in buffer.layout_runs() {
        if let Some(x) = run.cursor_position(&cursor) {
            return Some((
                x / hint,
                run.line_top / hint,
                (run.line_height + 1.0) / hint,
            ));
        }
    }

    Some((0.0, 0.0, (buffer.metrics().line_height + 1.0) / hint))
}

/// The document position nearest `point`, hit-tested against the rows the
/// last layout pass shaped.
///
/// A click above the first row or below the last snaps to that row rather
/// than being ignored, and a click past the end of a short line lands at
/// its end.
pub fn position_at(rows: &[ShapedRow], origin: Point, point: Point) -> Option<usize> {
    let y = point.y - origin.y;

    let row = rows
        .iter()
        .rev()
        .find(|row| y >= row.y)
        .or_else(|| rows.first())?;

    let hint = row.paragraph.hint_factor().unwrap_or(1.0);
    // Row positions are in screen (unhinted) space while the buffer's own
    // coordinates are hinted, which is the mirror of the division above.
    let cursor = row
        .paragraph
        .buffer()
        .hit((point.x - origin.x) * hint, (y - row.y) * hint)?;

    Some(row.mapping.to_source(cursor.index))
}
