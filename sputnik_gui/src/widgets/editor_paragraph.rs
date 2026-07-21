use std::any::Any;
use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;

use iced::advanced::Layout;
use iced::advanced::graphics::text::Paragraph as GraphicsParagraph;
use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::renderer::{self, Quad};
use iced::advanced::text::{self as text_advanced, Paragraph};
use iced::advanced::widget::{self, Tree, Widget};
use iced::widget::text::{Alignment, Catalog, Ellipsis, LineHeight, Shaping, Span, Wrapping};
use iced::{Background, Color, Element, Length, Pixels, Point, Rectangle, Size, alignment};

pub struct EditorParagraph<'a, Link, Theme, Renderer>
where
    Link: Clone + 'static,
    Renderer: text_advanced::Renderer,
    Theme: Catalog,
{
    spans: Box<dyn AsRef<[Span<'a, Link, Renderer::Font>]> + 'a>,
    show_line_numbers: bool,
    cursor: Option<usize>,
    size: Option<Pixels>,
    line_height: LineHeight,
    width: Length,
    height: Length,
    font: Option<Renderer::Font>,
    align_x: Alignment,
    align_y: alignment::Vertical,
    wrapping: Wrapping,
    ellipsis: Ellipsis,
    cursor_width: f32,
    nav_width: &'a Cell<f32>,
    class: Theme::Class<'a>,
}

/// One rendered row, shaped as its own independent [`Paragraph`].
///
/// Rows are cached by their text content (`key`), not by index: an edit
/// that inserts/removes a line shifts every following row's index but not
/// its text, so content-addressing lets unrelated rows keep their shaped
/// paragraph across the edit instead of being invalidated by the shift.
struct Row<P> {
    key: String,
    /// Byte offset of this row's first character within the flattened content.
    source_start: usize,
    paragraph: P,
    y: f32,
    height: f32,
}

struct State<P> {
    rows: Vec<Row<P>>,
}

impl<'a, Link, Theme, Renderer> EditorParagraph<'a, Link, Theme, Renderer>
where
    Link: Clone + 'static,
    Renderer: text_advanced::Renderer,
    Theme: Catalog,
    Renderer::Font: 'a,
{
    pub fn with_spans(
        spans: impl AsRef<[Span<'a, Link, Renderer::Font>]> + 'a,
        cursor: Option<usize>,
        nav_width: &'a Cell<f32>,
    ) -> Self {
        Self {
            spans: Box::new(spans),
            show_line_numbers: false,
            cursor,
            size: None,
            line_height: LineHeight::default(),
            width: Length::Shrink,
            height: Length::Shrink,
            font: None,
            align_x: Alignment::Default,
            align_y: alignment::Vertical::Top,
            wrapping: Wrapping::default(),
            ellipsis: Ellipsis::default(),
            cursor_width: 2.0,
            nav_width,
            class: Theme::default(),
        }
    }

    pub fn show_line_numbers(mut self, show: bool) -> Self {
        self.show_line_numbers = show;
        self
    }

    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = Some(size.into());
        self
    }
}

fn spans_total_len<Link, Font>(spans: &[Span<'_, Link, Font>]) -> usize {
    spans.iter().map(|s| s.text.len()).sum()
}

fn gutter_width() -> f32 {
    64.0
}

/// A span restricted to a byte range of its own text, preserving every
/// other attribute (style, link, ...).
fn sub_span<'a, Link: Clone, Font: Clone>(
    span: &Span<'a, Link, Font>,
    range: Range<usize>,
) -> Span<'a, Link, Font> {
    let text = match &span.text {
        Cow::Borrowed(s) => Cow::Borrowed(&s[range]),
        Cow::Owned(s) => Cow::Owned(s[range].to_string()),
    };
    Span {
        text,
        ..span.clone()
    }
}

/// Splits flat content into render rows at `\n` boundaries (one row per
/// rope line today). Kept as a single seam: a future fold/conceal/virtual-text
/// layer only needs to change what produces `(source_start, spans)` pairs,
/// not the caching or layout code below.
fn split_rows<'a, Link: Clone, Font: Clone>(
    spans: &[Span<'a, Link, Font>],
) -> Vec<(usize, Vec<Span<'a, Link, Font>>)> {
    let mut rows = Vec::new();
    let mut current = Vec::new();
    let mut row_start = 0usize;
    let mut offset = 0usize;

    for span in spans {
        let text: &str = &span.text;
        let mut piece_start = 0usize;
        for (i, ch) in text.char_indices() {
            if ch == '\n' {
                if i > piece_start {
                    current.push(sub_span(span, piece_start..i));
                }
                rows.push((row_start, std::mem::take(&mut current)));
                row_start = offset + i + 1;
                piece_start = i + 1;
            }
        }
        if piece_start < text.len() {
            current.push(sub_span(span, piece_start..text.len()));
        }
        offset += text.len();
    }
    rows.push((row_start, current));
    rows
}

fn row_key<Link, Font>(spans: &[Span<'_, Link, Font>]) -> String {
    let mut s = String::new();
    for span in spans {
        s.push_str(&span.text);
    }
    s
}

impl<Link, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for EditorParagraph<'_, Link, Theme, Renderer>
where
    Link: Clone + 'static,
    Renderer: text_advanced::Renderer + 'static,
    Renderer::Paragraph: Clone,
    Theme: Catalog,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State<Renderer::Paragraph>>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::<Renderer::Paragraph> { rows: Vec::new() })
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State<Renderer::Paragraph>>();
        let content = self.spans.as_ref().as_ref();
        let gutter = if self.show_line_numbers {
            gutter_width()
        } else {
            0.0
        };
        let rows = split_rows(content);

        layout::sized(limits, self.width, self.height, |limits| {
            let bounds = limits.max();
            let para_bounds = Size::new((bounds.width - gutter).max(0.0), bounds.height);

            self.nav_width.set(para_bounds.width);

            let size = self.size.unwrap_or_else(|| renderer.default_size());
            let font = self.font.unwrap_or_else(|| renderer.default_font());

            let desired = text_advanced::Text {
                content: (),
                bounds: para_bounds,
                size,
                line_height: self.line_height,
                font,
                align_x: self.align_x,
                align_y: self.align_y,
                shaping: Shaping::Advanced,
                wrapping: self.wrapping,
                ellipsis: self.ellipsis,
                hint_factor: renderer.scale_factor(),
            };

            // Index the previous frame's shaped rows by their text, so an
            // unchanged line reuses its paragraph (cheap Arc clone) instead
            // of being reshaped. Indexed by content, not position: inserting
            // or deleting a line shifts indices but not surviving rows' text.
            let previous = std::mem::take(&mut state.rows);
            let mut by_key: HashMap<&str, usize> = HashMap::with_capacity(previous.len());
            for (i, row) in previous.iter().enumerate() {
                by_key.entry(row.key.as_str()).or_insert(i);
            }

            let mut new_rows = Vec::with_capacity(rows.len());
            let mut y = 0.0f32;
            let mut width = 0.0f32;

            for (source_start, row_spans) in &rows {
                let key = row_key(row_spans);

                let cached = by_key
                    .get(key.as_str())
                    .map(|&i| previous[i].paragraph.clone());
                let (mut paragraph, mut needs_shape) = match cached {
                    Some(p) => (p, false),
                    None => (Renderer::Paragraph::default(), true),
                };

                if !needs_shape {
                    match paragraph.compare(desired) {
                        text_advanced::Difference::None => {}
                        text_advanced::Difference::Bounds => paragraph.resize(para_bounds),
                        text_advanced::Difference::Shape => needs_shape = true,
                    }
                }

                if needs_shape {
                    paragraph = Renderer::Paragraph::with_spans(text_advanced::Text {
                        content: row_spans.as_slice(),
                        bounds: para_bounds,
                        size,
                        line_height: self.line_height,
                        font,
                        align_x: self.align_x,
                        align_y: self.align_y,
                        shaping: Shaping::Advanced,
                        wrapping: self.wrapping,
                        ellipsis: self.ellipsis,
                        hint_factor: renderer.scale_factor(),
                    });
                }

                let min_bounds = paragraph.min_bounds();
                width = width.max(min_bounds.width);
                let height = min_bounds.height;

                new_rows.push(Row {
                    key,
                    source_start: *source_start,
                    paragraph,
                    y,
                    height,
                });
                y += height;
            }

            state.rows = new_rows;

            Size::new(width + gutter, y)
        })
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State<Renderer::Paragraph>>();
        let appearance = theme.style(&self.class);
        let bounds = layout.bounds();

        let total_height = state.rows.last().map(|r| r.y + r.height).unwrap_or(0.0);
        let total_width = state
            .rows
            .iter()
            .fold(0.0f32, |w, r| w.max(r.paragraph.min_bounds().width));
        let anchor = bounds.anchor(
            Size::new(total_width, total_height),
            self.align_x,
            self.align_y,
        );

        let content = self.spans.as_ref().as_ref();
        let gutter = if self.show_line_numbers {
            gutter_width()
        } else {
            0.0
        };
        let text_anchor = Point::new(anchor.x + gutter, anchor.y);

        let color = appearance.color.unwrap_or(style.text_color);

        // Draw line numbers in the gutter: one per row, at that row's top.
        if self.show_line_numbers {
            for (i, row) in state.rows.iter().enumerate() {
                let text = text_advanced::Text {
                    content: format!("{}", i + 1),
                    bounds: Size::new(gutter - 8.0, row.height),
                    size: self.size.unwrap_or_else(|| renderer.default_size()),
                    line_height: self.line_height,
                    font: self.font.unwrap_or_else(|| renderer.default_font()),
                    align_x: text_advanced::Alignment::Right,
                    align_y: alignment::Vertical::Top,
                    shaping: Shaping::Advanced,
                    wrapping: Wrapping::None,
                    ellipsis: Ellipsis::None,
                    hint_factor: None,
                };
                renderer.fill_text(
                    text,
                    // `Alignment::Right` anchors the text's *right* edge at
                    // `position.x`, not the box's left edge.
                    Point::new(anchor.x + gutter - 8.0, anchor.y + row.y),
                    Color::from_rgb(0.5, 0.5, 0.5),
                    *viewport,
                );
            }
        }

        for row in &state.rows {
            renderer.fill_paragraph(
                &row.paragraph,
                Point::new(text_anchor.x, text_anchor.y + row.y),
                color,
                *viewport,
            );
        }

        let total = spans_total_len(content);
        if let Some(cursor) = self.cursor
            && cursor <= total
        {
            draw_cursor(
                renderer,
                &state.rows,
                text_anchor,
                cursor,
                color,
                self.cursor_width,
            );
        }
    }
}

impl<'a, Link, Message, Theme, Renderer> From<EditorParagraph<'a, Link, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Link: Clone + 'static,
    Message: 'a,
    Renderer: text_advanced::Renderer + 'static + 'a,
    Renderer::Paragraph: Clone,
    Theme: Catalog + 'a,
{
    fn from(
        text: EditorParagraph<'a, Link, Theme, Renderer>,
    ) -> Element<'a, Message, Theme, Renderer> {
        Element::new(text)
    }
}

fn draw_cursor<Renderer: text_advanced::Renderer>(
    renderer: &mut Renderer,
    rows: &[Row<Renderer::Paragraph>],
    anchor: Point,
    cursor: usize,
    color: Color,
    width: f32,
) {
    // Last row whose `source_start` is <= cursor.
    let row_idx = match rows.binary_search_by_key(&cursor, |r| r.source_start) {
        Ok(i) => i,
        Err(0) => 0,
        Err(i) => i - 1,
    };
    let Some(row) = rows.get(row_idx) else { return };
    let local = cursor.saturating_sub(row.source_start);

    let hint = Paragraph::hint_factor(&row.paragraph).unwrap_or(1.0);
    let any: &dyn Any = &row.paragraph as &dyn Any;
    let Some(gp) = any.downcast_ref::<GraphicsParagraph>() else {
        return;
    };
    let Some((cx, cy, ch)) = cursor_pos_in_row(gp.buffer(), local, hint) else {
        return;
    };

    renderer.fill_quad(
        Quad {
            bounds: Rectangle {
                x: anchor.x + cx,
                y: anchor.y + row.y + cy,
                width,
                height: ch,
            },
            border: iced::Border::default(),
            shadow: iced::Shadow::default(),
            snap: true,
        },
        Background::Color(color),
    );
}

/// Cursor position within a single row's own buffer (always logical line 0,
/// possibly multiple wrapped visual runs).
fn cursor_pos_in_row(
    buffer: &cosmic_text::Buffer,
    local: usize,
    hint: f32,
) -> Option<(f32, f32, f32)> {
    let line_len = buffer.lines.first().map(|l| l.text().len()).unwrap_or(0);
    let cosmic_cursor = cosmic_text::Cursor::new(0, local.min(line_len));

    for run in buffer.layout_runs() {
        if let Some(x) = run.cursor_position(&cosmic_cursor) {
            return Some((
                x / hint,
                run.line_top / hint,
                (run.line_height + 1.0) / hint,
            ));
        }
    }

    let lh = (buffer.metrics().line_height + 1.0) / hint;
    Some((0.0, 0.0, lh))
}
