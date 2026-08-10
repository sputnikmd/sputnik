//! Shaping drawable rows, and keeping them across frames.

use std::ops::Range;

use iced::advanced::graphics::text::Paragraph;
use iced::advanced::text::{self, Paragraph as _};
use iced::widget::text::{Alignment, Ellipsis, LineHeight, Shaping, Span, Wrapping};
use iced::{Color, Font, Pixels, Size, alignment};

use crate::core::{Fingerprint, Layer, Mapping, Row, Style, Text};

/// One shaped row, ready to draw.
///
/// Rows are shaped a line at a time rather than as one large paragraph, so
/// only what is on screen is ever touched: drawing a hundred-thousand-line
/// file costs what drawing the forty visible lines costs.
pub struct ShapedRow {
    /// The line of the document this row renders, which is what lets a
    /// later pass find the same line's shaping by index.
    pub line: usize,
    /// The document bytes of the line this row renders.
    ///
    /// Taken from the line itself rather than from what ended up drawn, so
    /// that a caret inside text a layer hid still belongs to a row.
    pub source: Range<usize>,
    /// Translation between drawn offsets and document positions, which is
    /// what keeps clicks, the caret and selections honest once layers have
    /// hidden, added or substituted text.
    pub mapping: Mapping,
    /// The shaped text.
    pub paragraph: Paragraph,
    /// Viewport-relative vertical position: 0 is the top of the visible
    /// window, that is, the top of the scroll anchor's row.
    pub y: f32,
    /// Rendered height, wrapping included.
    pub height: f32,
    /// Summary of the styling this row was shaped with.
    fingerprint: Fingerprint,
}

impl ShapedRow {
    /// The bottom edge, viewport-relative.
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    /// The drawn text held inside the shaped paragraph, reused as a cache
    /// key instead of keeping a second copy of every visible line.
    fn drawn(&self) -> &str {
        self.paragraph
            .buffer()
            .lines
            .first()
            .map(|line| line.text())
            .unwrap_or_default()
    }
}

/// Everything needed to shape a row, gathered once per layout pass.
#[derive(Debug, Clone, Copy)]
pub struct Shaper {
    /// The area rows are wrapped and measured against.
    pub bounds: Size,
    /// Base font size, which a fragment's scale multiplies.
    pub size: Pixels,
    /// Height of one unwrapped row.
    pub line_height: LineHeight,
    /// Base font, from which bold and italic variants are derived.
    pub font: Font,
    /// How lines too wide for `bounds` are broken.
    pub wrapping: Wrapping,
    /// Device pixel ratio, when the renderer hints glyph positions.
    pub hint_factor: Option<f32>,
    /// Width of the line-number gutter.
    pub gutter: f32,
    /// Gap between a line number and the text it labels.
    pub gutter_padding: f32,
}

impl Shaper {
    /// The shaping parameters with no content, for asking a kept paragraph
    /// whether it is still valid.
    fn template(&self) -> text::Text<(), Font> {
        text::Text {
            content: (),
            bounds: self.bounds,
            size: self.size,
            line_height: self.line_height,
            font: self.font,
            align_x: Alignment::Default,
            align_y: alignment::Vertical::Top,
            shaping: Shaping::Advanced,
            wrapping: self.wrapping,
            ellipsis: Ellipsis::None,
            hint_factor: self.hint_factor,
        }
    }

    fn shape(&self, spans: &[Span<'_, (), Font>]) -> Paragraph {
        let template = self.template();
        Paragraph::with_spans(text::Text {
            content: spans,
            bounds: template.bounds,
            size: template.size,
            line_height: template.line_height,
            font: template.font,
            align_x: template.align_x,
            align_y: template.align_y,
            shaping: template.shaping,
            wrapping: template.wrapping,
            ellipsis: template.ellipsis,
            hint_factor: template.hint_factor,
        })
    }

    /// The shaping parameters for a line number: never wrapped, and
    /// unhinted so that it lands on the same pixel grid as the row it
    /// labels.
    ///
    /// Shaped left-aligned; the caller places it against the gutter's
    /// inner edge, since a paragraph is drawn from its own origin and
    /// carries no alignment of its own.
    pub fn gutter_template(&self, height: f32) -> text::Text<(), Font> {
        text::Text {
            content: (),
            bounds: Size::new(self.gutter - self.gutter_padding, height),
            size: self.size,
            line_height: self.line_height,
            font: self.font,
            align_x: Alignment::Default,
            align_y: alignment::Vertical::Top,
            shaping: Shaping::Advanced,
            wrapping: Wrapping::None,
            ellipsis: Ellipsis::None,
            hint_factor: None,
        }
    }

    /// Shapes one line number.
    pub fn shape_gutter(&self, number: &str, height: f32) -> Paragraph {
        let template = self.gutter_template(height);
        Paragraph::with_text(text::Text {
            content: number,
            bounds: template.bounds,
            size: template.size,
            line_height: template.line_height,
            font: template.font,
            align_x: template.align_x,
            align_y: template.align_y,
            shaping: template.shaping,
            wrapping: template.wrapping,
            ellipsis: template.ellipsis,
            hint_factor: template.hint_factor,
        })
    }

    /// One line's true rendered height, wrapping included.
    ///
    /// Scroll arithmetic needs this for lines that are not among the rows
    /// currently shaped. Assuming a single row's worth would under-count a
    /// wrapped line and leave the viewport jumping once it is shaped for
    /// real.
    pub fn measure<T: Text + ?Sized>(&self, text: &T, layer: &dyn Layer<T>, line: usize) -> f32 {
        let mut row = Row::new(line);
        layer.apply(text, &mut row);
        let mut spans = Vec::new();
        fill_spans(&row, self.font, self.size, &mut spans);
        self.shape(&spans).min_bounds().height
    }
}

/// Shapes the rows covering the viewport into `out`, starting at line
/// `anchor.0` scrolled `anchor.1` pixels up.
///
/// `previous` supplies already-shaped paragraphs to reuse, and `out` is
/// cleared first — pass the two as separate buffers and swap them, and a
/// reshape costs no allocation at all.
///
/// A candidate is tried at the index the line would occupy if nothing had
/// moved, which is a single comparison in the overwhelmingly common case of
/// scrolling or of nothing changing. Only when that misses does a scan run,
/// so that an edit inserting or deleting a line — which shifts every
/// following index but no content — still finds its rows.
///
/// A candidate matches when its [`Fingerprint`] agrees, which covers the
/// styling and the shape of the run boundaries, and when it draws exactly
/// the same bytes. The second check is made against the shaped text in
/// place, so two lines that happen to share a fingerprint can never swap
/// paragraphs and nothing is concatenated to find out.
pub fn shape_window<'a, T: Text + ?Sized>(
    text: &'a T,
    layer: &dyn Layer<T>,
    shaper: &Shaper,
    anchor: (usize, f32),
    previous: &[ShapedRow],
    out: &mut Vec<ShapedRow>,
) {
    let (anchor_line, anchor_offset) = anchor;
    out.clear();

    // One row and one span buffer serve every line, so a frame allocates
    // nothing per line beyond what the layers themselves ask for.
    let mut scratch = Row::new(anchor_line);
    let mut spans: Vec<Span<'a, (), Font>> = Vec::new();

    // The anchor row starts partly above the viewport when the offset is
    // non-zero; it, and only it, is clipped at the top.
    //
    // Whole-pixel positions are kept by snapping the start and advancing by
    // rounded heights, rather than by accumulating raw sub-pixel floats.
    // Rounding each row's absolute position independently would leave
    // neighbours a pixel apart whenever their rounding fell either side of
    // a half; this keeps the spacing between rows constant, which is what
    // reads as stable.
    let mut y = -anchor_offset.round();

    for line in anchor_line..text.line_count() {
        if y >= shaper.bounds.height {
            break;
        }

        scratch.reset(line);
        layer.apply(text, &mut scratch);

        let source = text.line_start(line)..text.line_end(line);
        let fingerprint = Fingerprint::of(&scratch);

        let paragraph = match reusable(previous, &scratch, line, fingerprint, shaper) {
            Some(paragraph) => paragraph,
            None => {
                fill_spans(&scratch, shaper.font, shaper.size, &mut spans);
                shaper.shape(&spans)
            }
        };

        let height = paragraph.min_bounds().height.round();
        let mapping = Mapping::new(&scratch, source.start);
        out.push(ShapedRow {
            line,
            source,
            mapping,
            paragraph,
            y,
            height,
            fingerprint,
        });
        y += height;
    }
}

/// A shaped paragraph from `previous` that can stand in for `row`.
fn reusable(
    previous: &[ShapedRow],
    row: &Row<'_>,
    line: usize,
    fingerprint: Fingerprint,
    shaper: &Shaper,
) -> Option<Paragraph> {
    let matches = |candidate: &ShapedRow| {
        candidate.fingerprint == fingerprint && draws(row, candidate.drawn())
    };

    let hint = previous
        .first()
        .and_then(|first| line.checked_sub(first.line))
        .and_then(|index| previous.get(index))
        .filter(|candidate| matches(candidate));

    let candidate = match hint {
        Some(candidate) => candidate,
        None => previous.iter().find(|candidate| matches(candidate))?,
    };

    let mut paragraph = candidate.paragraph.clone();
    match paragraph.compare(shaper.template()) {
        text::Difference::None => Some(paragraph),
        text::Difference::Bounds => {
            paragraph.resize(shaper.bounds);
            Some(paragraph)
        }
        text::Difference::Shape => None,
    }
}

/// Whether `row` would draw exactly `drawn`, compared run by run so that
/// nothing has to be concatenated to find out.
fn draws(row: &Row<'_>, drawn: &str) -> bool {
    let mut rest = drawn;
    for fragment in &row.fragments {
        match rest.strip_prefix(fragment.text.as_ref()) {
            Some(tail) => rest = tail,
            None => return false,
        }
    }
    rest.is_empty()
}

/// Translates the renderer-agnostic fragments of a row into iced spans.
///
/// The only place the two styling vocabularies meet: a front-end that is
/// not iced writes its own version of this and reuses everything else.
fn fill_spans<'a>(row: &Row<'a>, font: Font, size: Pixels, out: &mut Vec<Span<'a, (), Font>>) {
    out.clear();
    out.extend(row.fragments.iter().map(|fragment| {
        let Style {
            color,
            scale,
            bold,
            italic,
        } = fragment.style;

        Span {
            text: fragment.text.clone(),
            size: scale.map(|scale| Pixels(size.0 * scale)),
            line_height: None,
            font: (bold || italic).then_some(Font {
                weight: if bold {
                    iced::font::Weight::Bold
                } else {
                    font.weight
                },
                style: if italic {
                    iced::font::Style::Italic
                } else {
                    font.style
                },
                ..font
            }),
            color: color.map(|color| Color {
                r: color.r,
                g: color.g,
                b: color.b,
                a: color.a,
            }),
            link: None,
            highlight: None,
            padding: iced::Padding::ZERO,
            underline: false,
            strikethrough: false,
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Plain;

    fn shaper() -> Shaper {
        Shaper {
            bounds: Size::new(400.0, 400.0),
            size: Pixels(16.0),
            line_height: LineHeight::default(),
            font: Font::default(),
            wrapping: Wrapping::default(),
            hint_factor: None,
            gutter: 0.0,
            gutter_padding: 0.0,
        }
    }

    #[test]
    fn a_candidate_must_draw_exactly_the_same_bytes() {
        let text = String::from("hello");
        let mut row = Row::new(0);
        Plain.apply(&text, &mut row);

        assert!(draws(&row, "hello"));
        assert!(!draws(&row, "hell"), "a prefix is not a match");
        assert!(!draws(&row, "hello!"), "a longer string is not a match");

        row.split(2);
        assert!(
            draws(&row, "hello"),
            "where the runs are cut does not change what is drawn"
        );
    }

    /// Shaping is addressed by content, not by position, so an edit that
    /// renumbers every following line without changing any of them keeps
    /// their shaping. The line also has a twin with an identical
    /// fingerprint sitting at the index it moved to, so nothing but the
    /// byte comparison can tell them apart.
    #[test]
    fn a_line_that_only_shifted_keeps_its_shaping() {
        let shaper = shaper();
        let before = String::from("a\nb\nc\n");
        let mut previous = Vec::new();
        shape_window(&before, &Plain, &shaper, (0, 0.0), &[], &mut previous);
        assert!(previous.len() >= 3, "the fixture must shape every line");

        let after = String::from("new\na\nb\nc\n");
        let mut row = Row::new(1);
        Plain.apply(&after, &mut row);
        let fingerprint = Fingerprint::of(&row);

        assert_eq!(
            fingerprint, previous[1].fingerprint,
            "\"a\" and \"b\" must share a fingerprint, or this proves nothing"
        );
        assert!(
            reusable(&previous, &row, 1, fingerprint, &shaper).is_some(),
            "a line whose index shifted but whose content did not must keep \
             its shaping"
        );
    }

    #[test]
    fn a_line_whose_content_changed_is_reshaped() {
        let shaper = shaper();
        let before = String::from("a\nb\nc\n");
        let mut previous = Vec::new();
        shape_window(&before, &Plain, &shaper, (0, 0.0), &[], &mut previous);

        let after = String::from("zzz\nb\nc\n");
        let mut row = Row::new(0);
        Plain.apply(&after, &mut row);

        assert!(
            reusable(&previous, &row, 0, Fingerprint::of(&row), &shaper).is_none(),
            "nothing on screen draws \"zzz\", so it must be shaped afresh"
        );
    }
}
