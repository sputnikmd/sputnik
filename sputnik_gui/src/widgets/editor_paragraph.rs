use std::any::Any;
use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;

use iced::Event;
use iced::advanced::Layout;
use iced::advanced::Shell;
use iced::advanced::graphics::text::Paragraph as GraphicsParagraph;
use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::renderer::{self, Quad};
use iced::advanced::text::{self as text_advanced, Paragraph};
use iced::advanced::widget::{self, Tree, Widget};
use iced::widget::text::{Alignment, Catalog, Ellipsis, LineHeight, Shaping, Span, Wrapping};
use iced::{Background, Color, Element, Length, Pixels, Point, Rectangle, Size, alignment};
use ropey::Rope;

pub struct EditorParagraph<'a, Theme, Renderer>
where
    Renderer: text_advanced::Renderer,
    Theme: Catalog,
{
    buffer: &'a Rope,
    show_line_numbers: bool,
    cursor: Option<usize>,
    /// Selected byte range into the buffer, `start..end` with `start <=
    /// end`. Rendered as a layer beneath the text.
    selection: Option<Range<usize>>,
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
    /// `(line, offset)`: index of the topmost logical line currently
    /// rendered, plus how many pixels of that line are scrolled above the
    /// viewport (`0.0 <= offset < that line's rendered height`). Anchoring
    /// to a line rather than an absolute pixel position means the "how tall
    /// is an offscreen row" problem never comes up — only rows adjacent to
    /// what's already visible are ever shaped — while `offset` still gives
    /// smooth, pixel-precise scrolling within that.
    scroll_anchor: &'a Cell<(usize, f32)>,
    /// Set by `update()` when a wheel event is processed. `layout()` reads
    /// it to decide whether the scroll was user-initiated (wheel) vs
    /// cursor-initiated (keyboard): only in the latter case should
    /// scroll-into-view override the anchor.
    wheel_scrolled: bool,
    class: Theme::Class<'a>,
}

/// One rendered row, shaped as its own independent [`Paragraph`].
///
/// Rows are cached by their text content, not by index: an edit that
/// inserts/removes a line shifts every following row's index but not its
/// text, so content-addressing lets unrelated rows keep their shaped
/// paragraph across the edit instead of being invalidated by the shift. The
/// comparison text itself isn't stored here — `shaped_text` reads it back
/// out of the already-shaped `paragraph`, which owns a copy of it anyway.
struct Row<P> {
    /// Byte offset of this row's first character within the whole buffer.
    source_start: usize,
    paragraph: P,
    /// Byte length of this row's text, for the cursor offscreen check.
    text_len: usize,
    /// Viewport-relative (not document-relative): 0 is the top of the
    /// visible window, i.e. the top of `scroll_anchor`'s row.
    y: f32,
    height: f32,
}

struct State<P> {
    rows: Vec<Row<P>>,
    /// The `scroll_anchor` value this frame's `rows` were built from, so
    /// `draw` can recover each row's line number.
    anchor_line: usize,
}

impl<'a, Theme, Renderer> EditorParagraph<'a, Theme, Renderer>
where
    Renderer: text_advanced::Renderer,
    Theme: Catalog,
    Renderer::Font: 'a,
{
    pub fn new(
        buffer: &'a Rope,
        cursor: Option<usize>,
        nav_width: &'a Cell<f32>,
        scroll_anchor: &'a Cell<(usize, f32)>,
    ) -> Self {
        Self {
            buffer,
            show_line_numbers: false,
            cursor,
            selection: None,
            size: None,
            line_height: LineHeight::default(),
            width: Length::Fill,
            height: Length::Fill,
            font: None,
            align_x: Alignment::Default,
            align_y: alignment::Vertical::Top,
            wrapping: Wrapping::default(),
            ellipsis: Ellipsis::default(),
            cursor_width: 2.0,
            nav_width,
            scroll_anchor,
            wheel_scrolled: false,
            class: Theme::default(),
        }
    }

    pub fn show_line_numbers(mut self, show: bool) -> Self {
        self.show_line_numbers = show;
        self
    }

    pub fn selection(mut self, selection: Option<Range<usize>>) -> Self {
        self.selection = selection;
        self
    }

    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = Some(size.into());
        self
    }
}

fn gutter_width() -> f32 {
    64.0
}

/// A span restricted to a byte range of its own text, preserving every
/// other attribute (style, link, ...).
fn sub_span<'a, Font: Clone>(span: &Span<'a, (), Font>, range: Range<usize>) -> Span<'a, (), Font> {
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
fn split_rows<'a, Font: Clone>(
    spans: &[Span<'a, (), Font>],
) -> Vec<(usize, Vec<Span<'a, (), Font>>)> {
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

/// Builds an owned row-text string. Only needed as a fallback when a row's
/// spans can't be borrowed as one contiguous slice (see `row_text`).
fn row_key<Font>(spans: &[Span<'_, (), Font>]) -> String {
    let mut s = String::new();
    for span in spans {
        s.push_str(&span.text);
    }
    s
}

/// The row's text as a `Cow`: zero-copy in the common case where a row fits
/// entirely in one span (rope chunks are much larger than a typical line),
/// falling back to an owned concatenation only when a row straddles a chunk
/// boundary.
fn row_text<'a, Font: Clone>(spans: &'a [Span<'_, (), Font>]) -> Cow<'a, str> {
    match spans {
        [single] => Cow::Borrowed(single.text.as_ref()),
        _ => Cow::Owned(row_key(spans)),
    }
}

/// The text already shaped and stored inside a cached `Paragraph`, reused as
/// the cache's comparison key instead of keeping a second, redundant copy of
/// every visible line.
fn shaped_text<P: 'static>(paragraph: &P) -> Option<&str> {
    let any: &dyn Any = paragraph as &dyn Any;
    let gp = any.downcast_ref::<GraphicsParagraph>()?;
    gp.buffer().lines.first().map(|line| line.text())
}

/// Extracts render rows for a window of the buffer, starting at `start_line`
/// and covering at most `max_lines` logical lines (fewer near the end of the
/// buffer). `source_start` in each row is an absolute byte offset into the
/// whole buffer, so cursor lookups don't need to know where the window
/// begins. Zero-copy: spans borrow directly from the rope's storage chunks.
fn window_rows<'a, Font: Clone>(
    buffer: &'a Rope,
    start_line: usize,
    max_lines: usize,
) -> Vec<(usize, Vec<Span<'a, (), Font>>)> {
    let n_lines = buffer.len_lines();
    let start_line = start_line.min(n_lines.saturating_sub(1));
    let end_line = (start_line + max_lines).min(n_lines);

    let char_start = buffer.line_to_char(start_line);
    let char_end = if end_line < n_lines {
        buffer.line_to_char(end_line)
    } else {
        buffer.len_chars()
    };
    let byte_start = buffer.char_to_byte(char_start);

    let slice = buffer.slice(char_start..char_end);
    let spans: Vec<Span<'a, (), Font>> = slice
        .chunks()
        .filter(|c| !c.is_empty())
        .map(|chunk| Span::new(Cow::Borrowed(chunk)))
        .collect();

    split_rows(&spans)
        .into_iter()
        .map(|(local, row_spans)| (byte_start + local, row_spans))
        .collect()
}

/// Walks `(line, offset)` forward or backward by `delta_px` pixels of
/// scroll, using `height_of` for each line's true (wrap-aware) rendered
/// height. Positive `delta_px` scrolls down, negative scrolls up. Shared by
/// wheel-scroll and cursor-follow so a pixel of scroll means the same thing
/// to both, even across a wrapped row's true multi-visual-line height.
fn scroll_by(
    mut line: usize,
    offset: f32,
    delta_px: f32,
    n_lines: usize,
    viewport_height: f32,
    height_of: impl Fn(usize) -> f32,
) -> (usize, f32) {
    let mut offset = offset + delta_px;

    while offset < 0.0 {
        if line == 0 {
            offset = 0.0;
            break;
        }
        line -= 1;
        offset += height_of(line);
    }
    while offset >= 0.0 {
        let h = height_of(line);
        if line + 1 >= n_lines {
            // No further line to scroll into — but if this (possibly
            // wrapped) last line is itself taller than the viewport,
            // there's still more of *it* to reveal below what's currently
            // at the top, up to its own bottom.
            offset = offset.clamp(0.0, (h - viewport_height).max(0.0));
            break;
        }
        if offset < h {
            break;
        }
        offset -= h;
        line += 1;
    }

    (line, offset)
}

/// Shapes a single logical line in isolation to measure its true rendered
/// (wrap-aware) height, for scroll math that needs a line's height when
/// it's not among whatever's already shaped nearby.
fn measure_line_height<Renderer>(
    buffer: &Rope,
    line: usize,
    template: text_advanced::Text<(), Renderer::Font>,
) -> Option<f32>
where
    Renderer: text_advanced::Renderer,
{
    window_rows::<Renderer::Font>(buffer, line, 1)
        .first()
        .map(|(_, spans)| {
            Renderer::Paragraph::with_spans(text_advanced::Text {
                content: spans.as_slice(),
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
            .min_bounds()
            .height
        })
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for EditorParagraph<'_, Theme, Renderer>
where
    Renderer: text_advanced::Renderer + 'static,
    Renderer::Paragraph: Clone,
    Theme: Catalog,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State<Renderer::Paragraph>>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::<Renderer::Paragraph> {
            rows: Vec::new(),
            anchor_line: 0,
        })
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
        let gutter = if self.show_line_numbers {
            gutter_width()
        } else {
            0.0
        };

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

            // Clamp the anchor: the buffer may have shrunk (deletions) since
            // the last scroll. Once the anchor is the last line, force its
            // offset to 0 too — otherwise the last line could be scrolled
            // up past the top of the viewport, opening a gap of blank
            // space below it with nothing left to scroll into. The last
            // line should stop flush at the top instead.
            let n_lines = self.buffer.len_lines();
            let (mut anchor_line, mut anchor_offset) = self.scroll_anchor.get();
            anchor_line = anchor_line.min(n_lines.saturating_sub(1));
            if n_lines == 0 || anchor_line + 1 >= n_lines {
                anchor_offset = 0.0;
            }
            self.scroll_anchor.set((anchor_line, anchor_offset));

            // Logical lines needed to *safely* cover the viewport height,
            // assuming no wrapping. Wrapping only adds visual rows, so this
            // is always a sufficient (if sometimes generous) slice.
            let line_height_px = f32::from(self.line_height.to_absolute(size)).max(1.0);
            let max_visible_lines = ((para_bounds.height / line_height_px).ceil() as usize).max(1);

            // Shapes the visible window for a given anchor, reusing
            // `previous`'s paragraphs by content where possible. Called
            // once normally, and a second time if scroll-into-view (below)
            // has to move the anchor — content-addressed caching means the
            // second call is cheap when the two windows overlap.
            let shape_window = |anchor_line: usize,
                                 anchor_offset: f32,
                                 previous: &[Row<Renderer::Paragraph>]|
             -> (Vec<Row<Renderer::Paragraph>>, f32) {
                let windowed = window_rows::<Renderer::Font>(
                    self.buffer,
                    anchor_line,
                    max_visible_lines + 2,
                );

                // Index the previous frame's shaped rows by their text, so
                // an unchanged line reuses its paragraph (cheap Arc clone)
                // instead of being reshaped. Indexed by content, not
                // position: inserting or deleting a line shifts indices but
                // not surviving rows' text. Rows that scrolled offscreen
                // are simply never looked up again and drop here, which is
                // exactly the eviction we want.
                let mut by_key: HashMap<&str, usize> = HashMap::with_capacity(previous.len());
                for (i, row) in previous.iter().enumerate() {
                    if let Some(text) = shaped_text(&row.paragraph) {
                        by_key.entry(text).or_insert(i);
                    }
                }

                let mut new_rows = Vec::new();
                // The anchor row starts partially above the viewport when
                // `anchor_offset > 0`; it (and only it) gets clipped at the
                // top.
                //
                // `y` is kept a whole pixel at every step (snap the start,
                // then advance by a rounded height) rather than
                // accumulating raw sub-pixel floats. Rounding each row's
                // *absolute* position independently would still leave
                // neighboring rows off by a pixel from each other whenever
                // their rounding landed on opposite sides of .5 — this
                // keeps inter-row spacing constant instead, which is what
                // actually reads as stable to the eye.
                let mut y = -anchor_offset.round();
                let mut width = 0.0f32;

                for (source_start, row_spans) in &windowed {
                    // Stop shaping once the viewport is full: rows further
                    // down the window were only speculatively sliced from
                    // the rope, never touched, and cost nothing.
                    if y >= para_bounds.height {
                        break;
                    }

                    let text = row_text(row_spans);

                    let cached = by_key
                        .get(text.as_ref())
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
                    let height = min_bounds.height.round();

                    new_rows.push(Row {
                        source_start: *source_start,
                        paragraph,
                        text_len: text.len(),
                        y,
                        height,
                    });
                    y += height;
                }

                (new_rows, width)
            };

            let previous = std::mem::take(&mut state.rows);
            let (mut new_rows, mut width) = shape_window(anchor_line, anchor_offset, &previous);

            // Scroll into view: only when the scroll was NOT user-initiated
            // (mouse wheel). A wheel event already set the anchor exactly
            // where the user wants it; overriding it here would fight the
            // wheel and make scrolling impossible when the cursor is at a
            // different position in the document.
            if !self.wheel_scrolled
                && let Some(cursor) = self.cursor
            {
                let cursor_line = self
                    .buffer
                    .byte_to_line(cursor.min(self.buffer.len_bytes()));

                // Coarse jump: get the cursor's logical line roughly into
                // the shaped window. This is a row-count estimate
                // (imprecise once rows wrap to different heights) — the
                // pixel-precise pass below corrects any remaining error, so
                // it only needs to be close.
                if cursor_line < anchor_line {
                    anchor_line = cursor_line;
                    anchor_offset = 0.0;
                    (new_rows, width) = shape_window(anchor_line, anchor_offset, &new_rows);
                } else if cursor_line >= anchor_line + new_rows.len() {
                    anchor_line = cursor_line.saturating_sub(new_rows.len().saturating_sub(1));
                    anchor_offset = 0.0;
                    (new_rows, width) = shape_window(anchor_line, anchor_offset, &new_rows);
                }

                let mut cursor_idx = cursor_line
                    .checked_sub(anchor_line)
                    .filter(|&i| i < new_rows.len());

                // The coarse jump above reasons in row counts, so a window
                // of rows that wrap to unusual heights can still leave the
                // cursor's row just outside it. Force it in directly before
                // the pixel-precise pass.
                if cursor_idx.is_none() {
                    anchor_line = cursor_line;
                    anchor_offset = 0.0;
                    (new_rows, width) = shape_window(anchor_line, anchor_offset, &new_rows);
                    cursor_idx = Some(0);
                }

                // Pixel-precise pass: make sure the cursor's own row is
                // *fully* visible, not merely "the right logical line
                // landed somewhere in the shaped window". Catches a wrapped
                // row that's taller than the row-count estimate accounted
                // for, a row that grew past the bottom edge mid-keystroke
                // as it wrapped further while typing, and a row left
                // straddling the top or bottom edge from a previous wheel
                // scroll or a buffer edit elsewhere.
                if let Some(idx) = cursor_idx {
                    let row = &new_rows[idx];
                    let row_top = row.y;
                    let row_bottom = row.y + row.height;

                    // Only the anchor row (index 0) can ever start above
                    // the viewport: every later row's `y` is the running
                    // sum of preceding rows' true heights, so it's never
                    // negative.
                    let correction = if row_top < 0.0 {
                        row_top
                    } else if row_bottom > para_bounds.height {
                        row_bottom - para_bounds.height
                    } else {
                        0.0
                    };

                    if correction != 0.0 {
                        let height_of = |line: usize| -> f32 {
                            line.checked_sub(anchor_line)
                                .and_then(|i| new_rows.get(i))
                                .map(|r| r.height)
                                .or_else(|| {
                                    measure_line_height::<Renderer>(self.buffer, line, desired)
                                })
                                .unwrap_or(line_height_px)
                        };
                        (anchor_line, anchor_offset) = scroll_by(
                            anchor_line,
                            anchor_offset,
                            correction,
                            n_lines,
                            para_bounds.height,
                            height_of,
                        );
                        (new_rows, width) = shape_window(anchor_line, anchor_offset, &new_rows);
                    }
                }

                self.scroll_anchor.set((anchor_line, anchor_offset));
            }
            self.wheel_scrolled = false;

            let _ = width;
            state.rows = new_rows;
            state.anchor_line = anchor_line;

            bounds
        })
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let Event::Mouse(mouse::Event::WheelScrolled { delta }) = event else {
            return;
        };
        if !cursor.is_over(layout.bounds()) {
            return;
        }

        let size = self.size.unwrap_or_else(|| renderer.default_size());
        let font = self.font.unwrap_or_else(|| renderer.default_font());
        let line_height_px = f32::from(self.line_height.to_absolute(size)).max(1.0);

        let gutter = if self.show_line_numbers {
            gutter_width()
        } else {
            0.0
        };
        let bounds = layout.bounds();
        let para_bounds = Size::new((bounds.width - gutter).max(0.0), bounds.height);

        // Matches the sign convention of `iced::widget::scrollable`: negate
        // the raw wheel delta before applying it. `Lines` deltas are turned
        // into an equivalent pixel distance so both delta kinds share one
        // accumulation path below. Accumulating in pixels (rather than
        // rounding each event to whole lines) matters beyond smoothness:
        // touchpads/high-res mice under libinput send many small `Pixels`
        // events per gesture, each well under one line — rounding those to
        // the nearest line individually discarded almost every event.
        let delta_px = match *delta {
            mouse::ScrollDelta::Lines { y, .. } => -y * line_height_px * 3.0,
            mouse::ScrollDelta::Pixels { y, .. } => -y,
        };

        self.wheel_scrolled = true;

        if delta_px != 0.0 {
            let n_lines = self.buffer.len_lines();
            let state = tree.state.downcast_ref::<State<Renderer::Paragraph>>();

            let template = text_advanced::Text {
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

            // Prefer a row's actual shaped height when it's part of what's
            // currently on screen. Otherwise shape just that one line to
            // measure its *true* height rather than assuming one line's
            // worth: a wrapped row is several visual lines tall, and
            // treating it as one would under-count the scroll distance
            // needed to fully pass it, causing a visible jump once `layout`
            // later shapes it for real and its true height doesn't match
            // what this scroll math assumed. Bounded cost: a wheel event
            // only ever walks a handful of lines past what's cached.
            let height_of = |line: usize| -> f32 {
                line.checked_sub(state.anchor_line)
                    .and_then(|i| state.rows.get(i))
                    .map(|row| row.height)
                    .or_else(|| measure_line_height::<Renderer>(self.buffer, line, template))
                    .unwrap_or(line_height_px)
            };

            let (anchor_line, anchor_offset) = self.scroll_anchor.get();
            let new_anchor = scroll_by(
                anchor_line,
                anchor_offset,
                delta_px,
                n_lines,
                para_bounds.height,
                height_of,
            );
            if new_anchor != self.scroll_anchor.get() {
                self.scroll_anchor.set(new_anchor);
                shell.invalidate_layout();
                shell.request_redraw();
            }
        }

        shell.capture_event();
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
        let anchor = bounds.anchor(bounds.size(), self.align_x, self.align_y);

        let gutter = if self.show_line_numbers {
            gutter_width()
        } else {
            0.0
        };
        let text_anchor = Point::new(anchor.x + gutter, anchor.y);

        let color = appearance.color.unwrap_or(style.text_color);
        let size = self.size.unwrap_or_else(|| renderer.default_size());
        let line_height_px = f32::from(self.line_height.to_absolute(size)).max(1.0);

        // Rows are shaped independently of where they'll be drawn (see
        // `layout`), so a row scrolled partway off the top or bottom of
        // *this widget's own bounds* still renders its full, unclipped
        // height — without an explicit clip here, that overflow bleeds into
        // whatever is above/below us (padding, a sibling like the HUD)
        // instead of visibly cropping at our own edge. `with_layer` scopes
        // every draw call below (including `fill_quad`, which has no
        // per-call clip parameter of its own) to `bounds`.
        let clip = bounds.intersection(viewport).unwrap_or(bounds);
        renderer.with_layer(clip, |renderer| {
            // Selection is its own layer beneath the gutter and the text:
            // drawn first, so everything else paints on top of it.
            if let Some(selection) = &self.selection {
                draw_selection(renderer, &state.rows, text_anchor, selection);
            }

            // Draw line numbers in the gutter: one per row, at that row's top.
            if self.show_line_numbers {
                for (i, row) in state.rows.iter().enumerate() {
                    let text = text_advanced::Text {
                        content: format!("{}", state.anchor_line + i + 1),
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
                        clip,
                    );
                }
            }

            for row in &state.rows {
                renderer.fill_paragraph(
                    &row.paragraph,
                    Point::new(text_anchor.x, text_anchor.y + row.y),
                    color,
                    clip,
                );
            }

            if let Some(cursor) = self.cursor
                && cursor <= self.buffer.len_bytes()
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

            let (_, anchor_offset) = self.scroll_anchor.get();
            draw_scrollbar(
                renderer,
                bounds,
                state.rows.len(),
                self.buffer.len_lines(),
                state.anchor_line,
                anchor_offset,
                line_height_px,
            );
        });
    }
}

const SCROLLBAR_WIDTH: f32 = 6.0;
const SCROLLBAR_MIN_THUMB: f32 = 24.0;

/// Where the scrollbar thumb sits along the track, as a `0.0..=1.0`
/// fraction. Folding `anchor_offset` in (not just the discrete
/// `anchor_line`) is what makes the thumb track pixel-wise scrolling
/// smoothly instead of only moving once per whole line crossed.
fn scrollbar_fraction(
    anchor_line: usize,
    anchor_offset: f32,
    line_height_px: f32,
    n_lines: usize,
    visible_rows: usize,
) -> f32 {
    let max_scroll_lines = (n_lines - visible_rows).max(1) as f32;
    let effective_line = anchor_line as f32 + anchor_offset / line_height_px.max(1.0);
    (effective_line / max_scroll_lines).clamp(0.0, 1.0)
}

/// A minimal position/proportion indicator on the right edge, not a
/// scrollable control (not yet interactive). Sized and positioned purely
/// from line *counts* (visible rows vs. total lines) plus the sub-line
/// pixel offset, never from total document height, so it costs nothing
/// beyond what's already shaped for virtualization.
fn draw_scrollbar<Renderer: renderer::Renderer>(
    renderer: &mut Renderer,
    bounds: Rectangle,
    visible_rows: usize,
    n_lines: usize,
    anchor_line: usize,
    anchor_offset: f32,
    line_height_px: f32,
) {
    if n_lines == 0 || visible_rows >= n_lines {
        return;
    }

    let track_height = bounds.height;
    let thumb_height = (track_height * visible_rows as f32 / n_lines as f32)
        .max(SCROLLBAR_MIN_THUMB)
        .min(track_height);

    let scroll_fraction = scrollbar_fraction(
        anchor_line,
        anchor_offset,
        line_height_px,
        n_lines,
        visible_rows,
    );
    let thumb_y = bounds.y + scroll_fraction * (track_height - thumb_height);
    let track_x = bounds.x + bounds.width - SCROLLBAR_WIDTH;

    renderer.fill_quad(
        Quad {
            bounds: Rectangle {
                x: track_x,
                y: bounds.y,
                width: SCROLLBAR_WIDTH,
                height: track_height,
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
                x: track_x,
                y: thumb_y,
                width: SCROLLBAR_WIDTH,
                height: thumb_height,
            },
            border: iced::Border {
                radius: (SCROLLBAR_WIDTH / 2.0).into(),
                ..iced::Border::default()
            },
            shadow: iced::Shadow::default(),
            snap: false,
        },
        Background::Color(Color::from_rgba(0.5, 0.5, 0.5, 0.4)),
    );
}

impl<'a, Message, Theme, Renderer> From<EditorParagraph<'a, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Renderer: text_advanced::Renderer + 'static + 'a,
    Renderer::Paragraph: Clone,
    Theme: Catalog + 'a,
{
    fn from(text: EditorParagraph<'a, Theme, Renderer>) -> Element<'a, Message, Theme, Renderer> {
        Element::new(text)
    }
}

const SELECTION_COLOR: Color = Color::from_rgba(0.2, 0.5, 1.0, 0.35);

/// Highlight quads for the intersection of `selection` with each currently
/// rendered row. Purely additive over what's already shaped for
/// virtualization: reads each row's own `cosmic_text` buffer, never shapes
/// anything new.
fn draw_selection<Renderer: text_advanced::Renderer>(
    renderer: &mut Renderer,
    rows: &[Row<Renderer::Paragraph>],
    anchor: Point,
    selection: &Range<usize>,
) {
    if selection.start >= selection.end {
        return;
    }

    for row in rows {
        let row_end = row.source_start + row.text_len;
        if selection.end <= row.source_start || selection.start > row_end {
            continue;
        }

        let local_start = selection.start.saturating_sub(row.source_start);
        let local_end = selection
            .end
            .saturating_sub(row.source_start)
            .min(row.text_len);

        let hint = Paragraph::hint_factor(&row.paragraph).unwrap_or(1.0);
        let any: &dyn Any = &row.paragraph as &dyn Any;
        let Some(gp) = any.downcast_ref::<GraphicsParagraph>() else {
            continue;
        };
        let line_len = gp
            .buffer()
            .lines
            .first()
            .map(|l| l.text().len())
            .unwrap_or(0);
        let cursor_start = cosmic_text::Cursor::new(0, local_start.min(line_len));
        let cursor_end = cosmic_text::Cursor::new(0, local_end.min(line_len));

        for run in gp.buffer().layout_runs() {
            for (x, width) in run.highlight(cursor_start, cursor_end) {
                renderer.fill_quad(
                    Quad {
                        bounds: Rectangle {
                            x: anchor.x + x / hint,
                            y: anchor.y + row.y + run.line_top / hint,
                            width: width / hint,
                            height: (run.line_height + 1.0) / hint,
                        },
                        border: iced::Border::default(),
                        shadow: iced::Shadow::default(),
                        snap: true,
                    },
                    Background::Color(SELECTION_COLOR),
                );
            }
        }
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
    let (Some(first), Some(last)) = (rows.first(), rows.last()) else {
        return;
    };
    // Cursor is outside the currently rendered window (scrolled offscreen):
    // simply don't draw it, rather than clamping to the nearest edge.
    if cursor < first.source_start || cursor > last.source_start + last.text_len {
        return;
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for the wheel-scroll responsiveness bug: touchpads
    /// and high-res mice under libinput/Wayland send many `Pixels` deltas
    /// per gesture, each well under one line's height. Rounding each event
    /// to the nearest line individually (the old behavior) discarded almost
    /// all of them. Accumulating in pixels before converting to a line
    /// should still move the anchor once enough of them land.
    #[test]
    fn pixel_wheel_scroll_accumulates_small_deltas() {
        let mut text = String::new();
        for i in 1..=50 {
            text.push_str(&format!("line {i}\n"));
        }
        let buffer = Rope::from_str(&text);
        let nav_width = Cell::new(800.0);
        let scroll_anchor = Cell::new((0usize, 0.0f32));

        let element: iced::Element<'_, (), iced::Theme, iced::Renderer> =
            EditorParagraph::new(&buffer, None, &nav_width, &scroll_anchor)
                .size(24.0)
                .into();

        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced_test::core::Size::new(200.0, 200.0),
            element,
        );
        ui.point_at(iced_test::core::Point::new(50.0, 50.0));
        // Force an initial layout so `update` has shaped rows to read
        // heights back from.
        let _ = ui.snapshot(&iced::Theme::Light);

        // 20 events of 5px each = 100px total, well over one line height
        // (~31px at size 24), but each single event is far under half a
        // line, so the old `.round()`-per-event logic would drop every one.
        // Negative `y` scrolls down (see the sign convention note above).
        for _ in 0..20 {
            ui.simulate([iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Pixels { x: 0.0, y: -5.0 },
            })]);
        }
        let _ = ui.snapshot(&iced::Theme::Light);

        let (line, offset) = scroll_anchor.get();
        assert!(
            line > 0 || offset > 0.0,
            "20 events of 5px each should move the anchor, got (line={line}, offset={offset})"
        );
    }

    /// Regression test: scrolling down used to be unbounded, letting the
    /// last line scroll all the way up past the top of the viewport and
    /// leave blank space below it. It should stop with the last line
    /// flush at the top instead.
    #[test]
    fn scrolling_down_stops_with_last_line_flush_at_top() {
        let mut text = String::new();
        for i in 1..=10 {
            text.push_str(&format!("line {i}\n"));
        }
        let buffer = Rope::from_str(&text);
        let nav_width = Cell::new(800.0);
        let scroll_anchor = Cell::new((0usize, 0.0f32));

        let element: iced::Element<'_, (), iced::Theme, iced::Renderer> =
            EditorParagraph::new(&buffer, None, &nav_width, &scroll_anchor)
                .size(24.0)
                .into();

        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced_test::core::Size::new(200.0, 200.0),
            element,
        );
        ui.point_at(iced_test::core::Point::new(50.0, 50.0));
        let _ = ui.snapshot(&iced::Theme::Light);

        // Wildly overscroll: many large downward deltas, far more than the
        // document is tall.
        for _ in 0..20 {
            ui.simulate([iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Pixels { x: 0.0, y: -500.0 },
            })]);
        }
        let _ = ui.snapshot(&iced::Theme::Light);

        let n_lines = buffer.len_lines();
        assert_eq!(
            scroll_anchor.get(),
            (n_lines - 1, 0.0),
            "the last line should stop flush at the top, not scroll past it"
        );
    }

    /// Regression test: the scrollbar thumb used to be positioned purely
    /// from the discrete `anchor_line`, so it only moved once per whole
    /// line crossed instead of tracking pixel-wise scroll smoothly. Two
    /// different sub-line offsets on the *same* line must produce
    /// different fractions.
    #[test]
    fn scrollbar_fraction_reflects_sub_line_offset() {
        let at_top_of_line = scrollbar_fraction(2, 0.0, 31.2, 100, 10);
        let halfway_into_line = scrollbar_fraction(2, 15.6, 31.2, 100, 10);
        let at_next_line = scrollbar_fraction(3, 0.0, 31.2, 100, 10);

        assert!(
            at_top_of_line < halfway_into_line,
            "scrolling within a line should move the thumb, not just crossing lines"
        );
        assert!(
            halfway_into_line < at_next_line,
            "half a line of offset should sit strictly between the two line boundaries"
        );
    }

    /// Regression test: scrolling up used to assume every line is exactly
    /// one line tall when computing how far back to walk, since the line
    /// being entered is never part of the currently-rendered (forward-only)
    /// window and fell back to that estimate. For a row that word-wraps
    /// into several visual lines, that under-counted its real height,
    /// leaving `scroll_anchor` inconsistent with what `layout` discovers
    /// once it actually shapes the row — the visible symptom was a sudden
    /// jump in content right as a wrapped row scrolled into view.
    #[test]
    fn scrolling_up_into_a_wrapped_row_uses_its_true_height() {
        let long_line = "wrap ".repeat(40); // wraps into several visual lines in a narrow viewport
        let text = format!("short\n{long_line}\nend\nafter\n");
        let buffer = Rope::from_str(&text);
        let nav_width = Cell::new(100.0);
        let scroll_anchor = Cell::new((0usize, 0.0f32));

        let element: iced::Element<'_, (), iced::Theme, iced::Renderer> =
            EditorParagraph::new(&buffer, None, &nav_width, &scroll_anchor)
                .size(24.0)
                .into();

        // Narrow viewport forces `long_line` to wrap into multiple visual
        // lines instead of fitting on one.
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced_test::core::Size::new(100.0, 300.0),
            element,
        );
        ui.point_at(iced_test::core::Point::new(50.0, 50.0));
        let _ = ui.snapshot(&iced::Theme::Light);

        // Scroll all the way down: with the end-of-document clamp, the
        // anchor lands on the last line with zero offset regardless of how
        // large the delta is.
        ui.simulate([iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
            delta: iced::mouse::ScrollDelta::Pixels {
                x: 0.0,
                y: -10_000.0,
            },
        })]);
        let _ = ui.snapshot(&iced::Theme::Light);
        let n_lines = buffer.len_lines();
        assert_eq!(scroll_anchor.get(), (n_lines - 1, 0.0));

        // Scroll back up by enough to pass the three trailing single-line
        // rows (~31px each) and land inside the wrapped row (line 1). The
        // buggy version treated line 1 as one line tall too, so it kept
        // consuming budget past it and overshot all the way to line 0;
        // fixed, it stops on line 1 with an offset reflecting its real,
        // much larger wrapped height.
        ui.simulate([iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
            delta: iced::mouse::ScrollDelta::Pixels { x: 0.0, y: 100.0 },
        })]);
        let _ = ui.snapshot(&iced::Theme::Light);

        let (line, offset) = scroll_anchor.get();
        assert_eq!(
            line, 1,
            "should have stopped on the wrapped line, not overshot past it"
        );
        assert!(
            offset > 31.2,
            "offset {offset} should reflect line 1's true (wrapped, multi-line) height, \
             not a single line's height — using the wrong height is what caused the jump"
        );
    }
}
