//! Moving the viewport, and showing where it is.

use iced::advanced::renderer::{self, Quad};
use iced::{Background, Color, Rectangle};

/// Walks a `(line, offset)` anchor by `delta` pixels, asking `height_of`
/// for each line's true, wrap-aware rendered height. Positive scrolls down.
///
/// Shared by the wheel and by scroll-into-view so that a pixel of scroll
/// means the same thing to both, even across a wrapped row several visual
/// lines tall.
pub fn by(
    (mut line, offset): (usize, f32),
    delta: f32,
    lines: usize,
    viewport_height: f32,
    height_of: impl Fn(usize) -> f32,
) -> (usize, f32) {
    let mut offset = offset + delta;

    while offset < 0.0 {
        if line == 0 {
            offset = 0.0;
            break;
        }
        line -= 1;
        offset += height_of(line);
    }

    while offset >= 0.0 {
        let height = height_of(line);
        if line + 1 >= lines {
            // Nothing further to scroll into — but if this (possibly
            // wrapped) last line is itself taller than the viewport there
            // is still more of *it* below, down to its own bottom.
            offset = offset.clamp(0.0, (height - viewport_height).max(0.0));
            break;
        }
        if offset < height {
            break;
        }
        offset -= height;
        line += 1;
    }

    (line, offset)
}

const WIDTH: f32 = 6.0;
const MIN_THUMB: f32 = 24.0;

/// Where the thumb sits along its track, as a `0.0..=1.0` fraction.
///
/// Folding the sub-line offset in — not just the whole `line` — is what
/// makes the thumb track pixel-wise scrolling instead of only twitching
/// once per line crossed.
pub fn fraction(
    (line, offset): (usize, f32),
    line_height: f32,
    lines: usize,
    visible_rows: usize,
) -> f32 {
    let travel = (lines - visible_rows).max(1) as f32;
    let position = line as f32 + offset / line_height.max(1.0);
    (position / travel).clamp(0.0, 1.0)
}

/// A position indicator on the right edge — not (yet) a control you can
/// drag.
///
/// Sized purely from line *counts* plus the sub-line offset, never from a
/// total document height, so it costs nothing beyond what virtualization
/// has already measured.
pub fn draw<Renderer: renderer::Renderer>(
    renderer: &mut Renderer,
    bounds: Rectangle,
    anchor: (usize, f32),
    line_height: f32,
    lines: usize,
    visible_rows: usize,
) {
    if lines == 0 || visible_rows >= lines {
        return;
    }

    let track = bounds.height;
    let thumb = (track * visible_rows as f32 / lines as f32)
        .max(MIN_THUMB)
        .min(track);
    let y = bounds.y + fraction(anchor, line_height, lines, visible_rows) * (track - thumb);
    let x = bounds.x + bounds.width - WIDTH;

    renderer.fill_quad(
        Quad {
            bounds: Rectangle {
                x,
                y: bounds.y,
                width: WIDTH,
                height: track,
            },
            border: iced::Border::default(),
            shadow: iced::Shadow::default(),
            snap: false,
        },
        Background::Color(Color::from_rgba(0.5, 0.5, 0.5, 0.08)),
    );
    renderer.fill_quad(
        Quad {
            bounds: Rectangle {
                x,
                y,
                width: WIDTH,
                height: thumb,
            },
            border: iced::Border {
                radius: (WIDTH / 2.0).into(),
                ..iced::Border::default()
            },
            shadow: iced::Shadow::default(),
            snap: false,
        },
        Background::Color(Color::from_rgba(0.5, 0.5, 0.5, 0.4)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The thumb tracks scrolling pixel by pixel, not line by line: two
    /// different offsets within the same line must land at different
    /// fractions, or it would twitch once per line crossed and sit still
    /// in between.
    #[test]
    fn thumb_position_reflects_sub_line_offset() {
        let top = fraction((2, 0.0), 31.2, 100, 10);
        let halfway = fraction((2, 15.6), 31.2, 100, 10);
        let next = fraction((3, 0.0), 31.2, 100, 10);

        assert!(
            top < halfway,
            "scrolling within a line should move the thumb"
        );
        assert!(
            halfway < next,
            "half a line of offset belongs strictly between the two line boundaries"
        );
    }

    #[test]
    fn scrolling_down_stops_with_the_last_line_flush_at_the_top() {
        let anchor = by((0, 0.0), 10_000.0, 10, 200.0, |_| 30.0);
        assert_eq!(
            anchor,
            (9, 0.0),
            "overscrolling must stop at the last line rather than leaving \
             blank space below it"
        );
    }

    #[test]
    fn scrolling_up_uses_each_lines_true_height() {
        // Line 1 is wrapped and four rows tall; the rest are one row.
        let height_of = |line: usize| if line == 1 { 120.0 } else { 30.0 };
        let anchor = by((3, 0.0), -100.0, 4, 200.0, height_of);
        assert_eq!(
            anchor.0, 1,
            "should stop inside the wrapped line, not overshoot past it"
        );
        assert!(
            anchor.1 > 30.0,
            "the offset must reflect the wrapped line's real height ({} px), \
             or the viewport jumps once it is shaped for real",
            anchor.1
        );
    }

    #[test]
    fn a_last_line_taller_than_the_viewport_can_still_scroll_within_itself() {
        let anchor = by((0, 0.0), 500.0, 1, 100.0, |_| 300.0);
        assert_eq!(anchor, (0, 200.0), "stops at the line's own bottom edge");
    }
}
