//! The iced widget: a window onto a [`Document`], and nothing more.
//!
//! It draws what it is given and reports where the mouse landed. It never
//! edits — it holds `&Document`, so it cannot — and it never interprets a
//! key. What a press or a drag should mean is decided by the host, which
//! is what lets the same widget serve a conventional editor and a modal,
//! vim-style one without a line of this file changing.
//!
//! What gets drawn is decided by a [`Layer`] stack, which the widget only
//! runs. Styling, hiding and adding text all happen there.

mod paint;
mod row;
mod scroll;

use std::cell::Cell;

use iced::advanced::text::Renderer as _;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Layout, Renderer as _, Shell, layout, mouse, renderer, text};
use iced::widget::text::{Catalog, LineHeight, Wrapping};
use iced::{Color, Element, Event, Font, Length, Pixels, Point, Rectangle, Size};
use ropey::Rope;

use crate::core::{Document, Layer, Plain, Text};
use crate::editor::Viewport;
use iced::advanced::graphics::text::Paragraph;
use iced::advanced::text::Paragraph as _;
use row::{ShapedRow, Shaper};

/// The default layer stack, as a `'static` so a widget that was never
/// given one can still borrow a reference.
static PLAIN: Plain = Plain;

/// Width of the line-number gutter.
const GUTTER: f32 = 64.0;

/// Gap between a line number and the text it labels.
const GUTTER_PADDING: f32 = 8.0;

/// Shapes the line numbers for `rows`, reusing any that are still on
/// screen so that only numbers newly scrolled into view cost anything.
fn shape_gutter(
    rows: &[ShapedRow],
    shaper: &Shaper,
    previous: &[GutterRow],
    out: &mut Vec<GutterRow>,
) {
    out.clear();

    for row in rows {
        let line = row.line;
        let paragraph = previous
            .iter()
            .find(|number| number.line == line)
            .map(|number| number.paragraph.clone())
            .filter(|paragraph| {
                matches!(
                    paragraph.compare(shaper.gutter_template(row.height)),
                    text::Difference::None
                )
            })
            .unwrap_or_else(|| {
                let mut digits = [0; 20];
                shaper.shape_gutter(decimal(line + 1, &mut digits), row.height)
            });

        out.push(GutterRow { line, paragraph });
    }
}

/// Writes `value` into `buffer` and returns it as text, so a line number
/// costs no allocation.
fn decimal(mut value: usize, buffer: &mut [u8; 20]) -> &str {
    let mut index = buffer.len();
    loop {
        index -= 1;
        buffer[index] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 || index == 0 {
            break;
        }
    }
    std::str::from_utf8(&buffer[index..]).unwrap_or("?")
}

/// Shapes the window for `anchor` into `spare`, then swaps it into `rows`.
///
/// The two buffers trade places rather than one being rebuilt, so the
/// several passes a single layout may need share one pair of allocations.
fn reshape<T: Text + ?Sized>(
    text: &T,
    layer: &dyn Layer<T>,
    shaper: &Shaper,
    anchor: (usize, f32),
    rows: &mut Vec<ShapedRow>,
    spare: &mut Vec<ShapedRow>,
) {
    row::shape_window(text, layer, shaper, anchor, rows, spare);
    std::mem::swap(rows, spare);
}

/// What the widget reports to its host.
///
/// Deliberately raw: a resolved document position, and no opinion about
/// what it means. The host turns these into
/// [`Action`](crate::core::Action)s — conventionally
/// [`Motion::To`](crate::core::Motion::To) under
/// [`Action::Move`](crate::core::Action::Move) for a press and
/// [`Action::Select`](crate::core::Action::Select) for a drag, though
/// nothing here requires that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interaction {
    /// The left button went down over this position.
    Press(usize),
    /// The pointer moved to this position with the left button held.
    ///
    /// Published on every movement, including ones resolving to the
    /// position already reported. A captured event produces no message, so
    /// a host that stopped hearing about the drag would stop rebuilding
    /// the widget, and the selection would freeze halfway through.
    Drag(usize),
    /// The left button came back up, ending a drag.
    Release,
}

/// A view onto a [`Document`].
///
/// Build one with [`Editor::view`](crate::Editor::view), then wire up only
/// what is wanted: a mouse via [`TextEditor::on_interaction`], and what to
/// draw via [`TextEditor::layer`].
///
/// # Examples
///
/// ```
/// use sputnik_editor::{Editor, Interaction};
///
/// #[derive(Debug, Clone)]
/// enum Message {
///     Editor(Interaction),
/// }
///
/// let editor = Editor::<String>::from_str("hello");
/// let widget = editor
///     .view()
///     .on_interaction(Message::Editor)
///     .show_line_numbers(true)
///     .size(18.0);
///
/// let _: iced::Element<'_, Message> = widget.into();
/// ```
pub struct TextEditor<'a, Message, T: Text = Rope, Theme = iced::Theme>
where
    Theme: Catalog,
{
    document: &'a Document<T>,
    /// Written during layout with what only rendering can measure, and
    /// read back by the host for wrap-aware motions. See [`Viewport`].
    viewport: &'a Cell<Viewport>,
    layer: &'a dyn Layer<T>,
    show_line_numbers: bool,
    size: Option<Pixels>,
    line_height: LineHeight,
    width: Length,
    height: Length,
    font: Option<Font>,
    wrapping: Wrapping,
    cursor_width: f32,
    on_interaction: Option<Box<dyn Fn(Interaction) -> Message + 'a>>,
    /// Set when a wheel event moved the viewport, and read by the layout
    /// pass that follows it to leave the anchor alone. A wheel has already
    /// put the viewport exactly where the user wants it, and following the
    /// caret instead would make scrolling away from it impossible.
    wheel_scrolled: bool,
    class: Theme::Class<'a>,
}

struct State {
    rows: Vec<ShapedRow>,
    /// The buffer the previous pass shaped into. Reshaping fills this one
    /// and swaps, so repeated passes within a single layout reuse two
    /// allocations between them instead of making a new one each time.
    spare: Vec<ShapedRow>,
    /// Line numbers, shaped once and kept for as long as they stay on
    /// screen. Drawing them as text instead would reshape every digit on
    /// every frame.
    gutter: Vec<GutterRow>,
    /// The same double buffer as `spare`, for the gutter.
    gutter_spare: Vec<GutterRow>,
    /// The anchor line `rows` were built from, so `draw` can recover each
    /// row's line number.
    anchor_line: usize,
    /// The caret as of the last layout pass.
    ///
    /// Kept here rather than on the widget, which `view()` rebuilds every
    /// frame, so scroll-into-view can tell "the caret moved" from "this is
    /// just another layout pass". Without it, a relayout caused by
    /// something else entirely — a mouse moving over an unrelated part of
    /// the UI — would yank the viewport back to a caret the user never
    /// touched.
    last_cursor: Option<usize>,
    /// Whether the left button is down. Survives the widget being rebuilt
    /// between the press and the release, for the same reason as above.
    dragging: bool,
}

/// One shaped line number.
struct GutterRow {
    line: usize,
    paragraph: Paragraph,
}

impl<'a, Message, T: Text, Theme> TextEditor<'a, Message, T, Theme>
where
    Theme: Catalog,
{
    /// A view onto `document`, publishing its measurements into `viewport`.
    pub fn new(document: &'a Document<T>, viewport: &'a Cell<Viewport>) -> Self {
        Self {
            document,
            viewport,
            layer: &PLAIN,
            show_line_numbers: false,
            size: None,
            line_height: LineHeight::default(),
            width: Length::Fill,
            height: Length::Fill,
            font: None,
            wrapping: Wrapping::default(),
            cursor_width: 2.0,
            on_interaction: None,
            wheel_scrolled: false,
            class: Theme::default(),
        }
    }

    /// Gives the widget a mouse.
    ///
    /// Leave it unset and mouse events are not merely ignored but not even
    /// captured, so a host that drives everything from the keyboard keeps
    /// them for itself.
    pub fn on_interaction(mut self, f: impl Fn(Interaction) -> Message + 'a) -> Self {
        self.on_interaction = Some(Box::new(f));
        self
    }

    /// Decides what gets drawn.
    ///
    /// Defaults to [`Plain`], the document as stored. Anything richer is a
    /// stack whose bottom layer is `Plain`.
    ///
    /// ```
    /// use sputnik_editor::core::{Layer, Plain, Row, Style, Text};
    /// use sputnik_editor::Editor;
    ///
    /// struct Marker;
    ///
    /// impl<T: Text + ?Sized> Layer<T> for Marker {
    ///     fn apply<'a>(&self, text: &'a T, row: &mut Row<'a>) {
    ///         row.insert(text.line_start(row.line), "| ", Style::default());
    ///     }
    /// }
    ///
    /// let stack = (Plain, Marker);
    /// let editor = Editor::<String>::from_str("quoted");
    /// let widget = editor.view::<()>().layer(&stack);
    /// ```
    pub fn layer(mut self, layer: &'a dyn Layer<T>) -> Self {
        self.layer = layer;
        self
    }

    /// Draws line numbers in a gutter to the left of the text.
    pub fn show_line_numbers(mut self, show: bool) -> Self {
        self.show_line_numbers = show;
        self
    }

    /// Sets the base font size. Fragment scales multiply it.
    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = Some(size.into());
        self
    }

    /// Sets the base font, from which bold and italic are derived.
    pub fn font(mut self, font: impl Into<Font>) -> Self {
        self.font = Some(font.into());
        self
    }

    /// Sets the height of one unwrapped row.
    pub fn line_height(mut self, line_height: impl Into<LineHeight>) -> Self {
        self.line_height = line_height.into();
        self
    }

    fn gutter(&self) -> f32 {
        if self.show_line_numbers { GUTTER } else { 0.0 }
    }

    /// Top-left corner of the text itself, past the gutter.
    fn text_origin(&self, bounds: Rectangle) -> Point {
        Point::new(bounds.x + self.gutter(), bounds.y)
    }

    /// Reports `interaction`, if the host asked for any.
    fn report(&self, interaction: Interaction, shell: &mut Shell<'_, Message>) {
        if let Some(on_interaction) = self.on_interaction.as_ref() {
            shell.publish(on_interaction(interaction));
        }
    }
}

impl<Message, T: Text, Theme> Widget<Message, Theme, iced::Renderer>
    for TextEditor<'_, Message, T, Theme>
where
    Theme: Catalog,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State {
            rows: Vec::new(),
            spare: Vec::new(),
            gutter: Vec::new(),
            gutter_spare: Vec::new(),
            anchor_line: 0,
            last_cursor: None,
            dragging: false,
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
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State>();
        let text = self.document.text_storage();
        let gutter = self.gutter();

        layout::sized(limits, self.width, self.height, |limits| {
            let bounds = limits.max();
            let text_bounds = Size::new((bounds.width - gutter).max(0.0), bounds.height);

            let shaper = Shaper {
                bounds: text_bounds,
                size: self.size.unwrap_or_else(|| renderer.default_size()),
                line_height: self.line_height,
                font: self.font.unwrap_or_else(|| renderer.default_font()),
                wrapping: self.wrapping,
                hint_factor: renderer.scale_factor(),
                gutter: GUTTER,
                gutter_padding: GUTTER_PADDING,
            };
            let line_height = f32::from(self.line_height.to_absolute(shaper.size)).max(1.0);

            // Clamp the anchor: the document may have shrunk since the last
            // scroll. Once it is on the last line force its offset to zero
            // too, or that line could scroll up past the top and open a gap
            // of blank space with nothing left to scroll into.
            let lines = text.line_count();
            let (mut line, mut offset) = self.viewport.get().scroll;
            line = line.min(lines.saturating_sub(1));
            if lines == 0 || line + 1 >= lines {
                offset = 0.0;
            }
            let mut anchor = (line, offset);

            let mut rows = std::mem::take(&mut state.rows);
            let mut spare = std::mem::take(&mut state.spare);
            reshape(text, self.layer, &shaper, anchor, &mut rows, &mut spare);

            // Scroll into view — but only when the scroll was not the
            // user's own doing (the wheel already put the viewport exactly
            // where they want it) and the caret actually moved.
            let cursor = self.document.cursor();
            let cursor_moved = Some(cursor) != state.last_cursor;
            state.last_cursor = Some(cursor);

            if !self.wheel_scrolled && cursor_moved {
                let cursor_line = text.line_of(cursor);

                // Coarse jump, in row counts: gets the caret's line roughly
                // into the shaped window. Imprecise once rows wrap to
                // different heights, which the pixel pass below corrects.
                if cursor_line < anchor.0 {
                    anchor = (cursor_line, 0.0);
                    reshape(text, self.layer, &shaper, anchor, &mut rows, &mut spare);
                } else if cursor_line >= anchor.0 + rows.len() {
                    anchor = (
                        cursor_line.saturating_sub(rows.len().saturating_sub(1)),
                        0.0,
                    );
                    reshape(text, self.layer, &shaper, anchor, &mut rows, &mut spare);
                }

                let mut index = cursor_line
                    .checked_sub(anchor.0)
                    .filter(|&index| index < rows.len());

                // Row counts can still leave the caret's row just outside a
                // window of unusually tall rows. Force it in.
                if index.is_none() {
                    anchor = (cursor_line, 0.0);
                    reshape(text, self.layer, &shaper, anchor, &mut rows, &mut spare);
                    index = Some(0);
                }

                // Pixel-precise pass: the caret's row must be *fully*
                // visible, not merely "the right line is somewhere in the
                // window". Catches a wrapped row taller than the row-count
                // estimate allowed for, a row that grew past the bottom
                // edge mid-keystroke as it wrapped further, and a row left
                // straddling an edge by an earlier scroll.
                if let Some(index) = index {
                    let (top, bottom) = (rows[index].y, rows[index].bottom());
                    // Only the anchor row can start above the viewport:
                    // every later row's `y` is a running sum of true
                    // heights and so is never negative.
                    let correction = if top < 0.0 {
                        top
                    } else if bottom > text_bounds.height {
                        bottom - text_bounds.height
                    } else {
                        0.0
                    };

                    if correction != 0.0 {
                        let anchor_line = anchor.0;
                        let height_of = |line: usize| {
                            line.checked_sub(anchor_line)
                                .and_then(|index| rows.get(index))
                                .map(|row| row.height)
                                .unwrap_or_else(|| shaper.measure(text, self.layer, line))
                        };
                        anchor =
                            scroll::by(anchor, correction, lines, text_bounds.height, height_of);
                        reshape(text, self.layer, &shaper, anchor, &mut rows, &mut spare);
                    }
                }
            }
            self.wheel_scrolled = false;

            // How many rows a page is worth. When the shaped window fills
            // the viewport its row count is exact, wrapping and all. When
            // the document is shorter than the screen there simply are not
            // enough rows to count — and counting them anyway would make a
            // page-down on a two-line document travel two rows forever
            // after, since the measurement outlives the document that
            // produced it.
            let visible_rows = if rows
                .last()
                .is_some_and(|row| row.bottom() >= text_bounds.height)
            {
                rows.len()
            } else {
                ((text_bounds.height / line_height).ceil() as usize).max(1)
            };

            self.viewport.set(Viewport {
                scroll: anchor,
                wrap_width: text_bounds.width,
                font_size: shaper.size.0,
                line_height,
                visible_rows,
            });

            if self.show_line_numbers {
                let mut numbers = std::mem::take(&mut state.gutter);
                let mut numbers_spare = std::mem::take(&mut state.gutter_spare);
                shape_gutter(&rows, &shaper, &numbers, &mut numbers_spare);
                std::mem::swap(&mut numbers, &mut numbers_spare);
                state.gutter = numbers;
                state.gutter_spare = numbers_spare;
            }

            state.anchor_line = anchor.0;
            state.rows = rows;
            state.spare = spare;

            bounds
        })
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let origin = self.text_origin(bounds);

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                // No callback means the mouse is not merely ignored but
                // left entirely alone, event uncaptured.
                if self.on_interaction.is_none() {
                    return;
                }
                let state = tree.state.downcast_mut::<State>();
                let Some(point) = cursor.position_over(bounds) else {
                    return;
                };
                let Some(position) = paint::position_at(&state.rows, origin, point) else {
                    return;
                };
                state.dragging = true;
                self.report(Interaction::Press(position), shell);
                shell.capture_event();
            }

            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let state = tree.state.downcast_ref::<State>();
                if !state.dragging || self.on_interaction.is_none() {
                    return;
                }
                // Follows the pointer past the widget's own bounds: a drag
                // routinely overshoots while selecting, and it should keep
                // tracking rather than stall at the edge.
                let Some(point) = cursor.position() else {
                    return;
                };
                let Some(position) = paint::position_at(&state.rows, origin, point) else {
                    return;
                };
                self.report(Interaction::Drag(position), shell);
                shell.capture_event();
            }

            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let state = tree.state.downcast_mut::<State>();
                if !std::mem::take(&mut state.dragging) {
                    return;
                }
                self.report(Interaction::Release, shell);
                shell.capture_event();
            }

            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if !cursor.is_over(bounds) {
                    return;
                }
                self.scroll(tree, *delta, bounds, renderer, shell);
                shell.capture_event();
            }

            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let origin = self.text_origin(bounds);

        let color = theme.style(&self.class).color.unwrap_or(style.text_color);
        let size = self.size.unwrap_or_else(|| renderer.default_size());
        let line_height = f32::from(self.line_height.to_absolute(size)).max(1.0);

        // Rows are shaped independently of where they land, so one scrolled
        // partway off an edge still renders its full height. Without an
        // explicit clip that overflow bleeds into whatever sits above or
        // below us rather than cropping at our own edge. `with_layer`
        // scopes everything below — including `fill_quad`, which has no
        // clip parameter of its own.
        let clip = bounds.intersection(viewport).unwrap_or(bounds);
        renderer.with_layer(clip, |renderer| {
            // Beneath everything else, so the text paints on top of it.
            let selection = self.document.selection();
            if !selection.is_empty() {
                paint::selection(renderer, &state.rows, origin, selection.range());
            }

            for (number, row) in state.gutter.iter().zip(&state.rows) {
                // A paragraph is drawn from its own origin and carries no
                // alignment, so the digits are pushed right here: their
                // right edge sits one padding clear of the text.
                let width = number.paragraph.min_bounds().width;
                renderer.fill_paragraph(
                    &number.paragraph,
                    Point::new(bounds.x + GUTTER - GUTTER_PADDING - width, bounds.y + row.y),
                    Color::from_rgb(0.5, 0.5, 0.5),
                    clip,
                );
            }

            for row in &state.rows {
                renderer.fill_paragraph(
                    &row.paragraph,
                    Point::new(origin.x, origin.y + row.y),
                    color,
                    clip,
                );
            }

            paint::cursor(
                renderer,
                &state.rows,
                origin,
                self.document.cursor(),
                color,
                self.cursor_width,
            );

            scroll::draw(
                renderer,
                bounds,
                self.viewport.get().scroll,
                line_height,
                self.document.text_storage().line_count(),
                state.rows.len(),
            );
        });
    }
}

impl<Message, T: Text, Theme> TextEditor<'_, Message, T, Theme>
where
    Theme: Catalog,
{
    fn scroll(
        &mut self,
        tree: &Tree,
        delta: mouse::ScrollDelta,
        bounds: Rectangle,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
    ) {
        let text = self.document.text_storage();
        let size = self.size.unwrap_or_else(|| renderer.default_size());
        let line_height = f32::from(self.line_height.to_absolute(size)).max(1.0);

        // Matches `iced::widget::scrollable`'s sign convention: negate the
        // raw delta. `Lines` are converted to pixels so both kinds share
        // one accumulation path — and accumulating in pixels matters
        // beyond smoothness, because touchpads send many small `Pixels`
        // events per gesture, each well under one line. Rounding those to
        // whole lines individually discarded almost every one of them.
        let delta = match delta {
            mouse::ScrollDelta::Lines { y, .. } => -y * line_height * 3.0,
            mouse::ScrollDelta::Pixels { y, .. } => -y,
        };

        // Even a delta of zero counts as a wheel scroll: the flag is what
        // tells the next layout pass not to drag the viewport back to the
        // caret.
        self.wheel_scrolled = true;
        if delta == 0.0 {
            return;
        }

        let state = tree.state.downcast_ref::<State>();
        let text_bounds = Size::new((bounds.width - self.gutter()).max(0.0), bounds.height);
        let shaper = Shaper {
            bounds: text_bounds,
            size,
            line_height: self.line_height,
            font: self.font.unwrap_or_else(|| renderer.default_font()),
            wrapping: self.wrapping,
            hint_factor: renderer.scale_factor(),
            gutter: GUTTER,
            gutter_padding: GUTTER_PADDING,
        };

        // Prefer a row's real shaped height when it is already on screen.
        // Otherwise shape that one line to measure its *true* height rather
        // than assuming a single line's worth: a wrapped row is several
        // visual lines tall, and under-counting it makes the content jump
        // once layout shapes it for real. Bounded — a wheel event only ever
        // walks a few lines past what is cached.
        let anchor_line = state.anchor_line;
        let height_of = |line: usize| {
            line.checked_sub(anchor_line)
                .and_then(|index| state.rows.get(index))
                .map(|row| row.height)
                .unwrap_or_else(|| shaper.measure(text, self.layer, line))
        };

        let viewport = self.viewport.get();
        let scrolled = scroll::by(
            viewport.scroll,
            delta,
            text.line_count(),
            text_bounds.height,
            height_of,
        );

        if scrolled != viewport.scroll {
            self.viewport.set(Viewport {
                scroll: scrolled,
                ..viewport
            });
            shell.invalidate_layout();
            shell.request_redraw();
        }
    }
}

impl<'a, Message, T: Text, Theme> From<TextEditor<'a, Message, T, Theme>>
    for Element<'a, Message, Theme, iced::Renderer>
where
    Message: 'a,
    T: 'a,
    Theme: Catalog + 'a,
{
    fn from(editor: TextEditor<'a, Message, T, Theme>) -> Self {
        Element::new(editor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Action, Edit, Layer, Motion, Plain, Row, Selection, Text};
    use crate::editor::Editor;
    use iced_test::core::renderer::Headless;
    use iced_test::runtime::UserInterface;
    use iced_test::runtime::user_interface::Cache;

    const SIZE: f32 = 24.0;

    fn editor(text: &str) -> Editor {
        Editor::from_str(text)
    }

    fn lines(count: usize) -> String {
        (1..=count).map(|i| format!("line {i}\n")).collect()
    }

    fn renderer() -> iced::Renderer {
        iced_test::futures::futures::executor::block_on(<iced::Renderer as Headless>::new(
            iced_test::core::renderer::Settings::default(),
            None,
        ))
        .expect("create headless renderer")
    }

    fn simulator(
        editor: &Editor,
        size: iced::Size,
    ) -> iced_test::Simulator<'_, (), iced::Theme, iced::Renderer> {
        let element: iced::Element<'_, (), iced::Theme, iced::Renderer> =
            editor.view().size(SIZE).show_line_numbers(true).into();
        iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced_test::core::Size::new(size.width, size.height),
            element,
        )
    }

    /// The host's own translation of an [`Interaction`], mirroring what
    /// `sputnik_gui` does — a press places the caret, a drag extends the
    /// selection.
    fn apply(editor: &mut Editor, interaction: Interaction) {
        match interaction {
            Interaction::Press(at) => editor.perform(Action::Move(Motion::To(at))),
            Interaction::Drag(at) => editor.perform(Action::Select(Motion::To(at))),
            Interaction::Release => {}
        }
    }

    /// Drives one frame of a real host loop: builds a fresh widget (as
    /// every redraw does), feeds it `events`, applies whatever it published
    /// back onto `editor`, and returns the retained cache plus the messages
    /// for the next frame.
    ///
    /// Going through a full `view -> event -> message -> apply -> redraw`
    /// cycle is the only honest way to exercise this widget now that it
    /// cannot mutate the document itself.
    fn frame(
        editor: &mut Editor,
        size: iced::Size,
        cache: Cache,
        renderer: &mut iced::Renderer,
        events: &[iced::Event],
        cursor: mouse::Cursor,
    ) -> (Cache, Vec<Interaction>) {
        let element: iced::Element<'_, Interaction, iced::Theme, iced::Renderer> = editor
            .view()
            .size(SIZE)
            .show_line_numbers(true)
            .on_interaction(|interaction| interaction)
            .into();
        let mut interface = UserInterface::build(element, size, cache, renderer);

        let mut messages = Vec::new();
        let _ = interface.update(
            &iced_test::core::window::Headless,
            &iced_test::core::shell::Waker::noop(),
            events,
            cursor,
            renderer,
            &mut messages,
        );
        // Drawing reflects this frame's pre-update state, exactly as in
        // production: the interface still borrows `editor` here, so the
        // messages cannot be applied until `into_cache()` releases it.
        interface.draw(renderer, &iced::Theme::Light, &Default::default(), cursor);
        let cache = interface.into_cache();

        for interaction in &messages {
            apply(editor, *interaction);
        }
        (cache, messages)
    }

    /// The guarantee this whole design exists to provide: a host that never
    /// calls `.on_interaction(..)` gets a mouse that does nothing at all —
    /// not even capturing the event — so a keyboard-driven or modal host
    /// can own mouse handling entirely.
    #[test]
    fn the_mouse_is_inert_without_a_callback() {
        let editor = editor("hello\nworld\n");
        let size = iced::Size::new(300.0, 200.0);
        let mut renderer = renderer();

        let element: iced::Element<'_, (), iced::Theme, iced::Renderer> =
            editor.view().size(SIZE).into();
        let mut interface = UserInterface::build(element, size, Cache::default(), &mut renderer);
        let cursor = mouse::Cursor::Available(iced::Point::new(250.0, 45.0));
        interface.draw(
            &mut renderer,
            &iced::Theme::Light,
            &Default::default(),
            cursor,
        );

        let mut messages: Vec<()> = Vec::new();
        let (_, statuses) = interface.update(
            &iced_test::core::window::Headless,
            &iced_test::core::shell::Waker::noop(),
            &[
                iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ],
            cursor,
            &mut renderer,
            &mut messages,
        );

        assert!(
            messages.is_empty(),
            "no callback, so nothing may be published"
        );
        assert!(
            statuses.iter().all(|s| *s == iced::event::Status::Ignored),
            "mouse events must not be captured either, or a host could not \
             fall back to handling them itself"
        );
        assert_eq!(editor.cursor(), 0, "the caret must not move");
        assert!(editor.selection().is_empty());
    }

    #[test]
    fn a_press_reports_the_position_under_the_pointer() {
        let mut editor = editor("hello\nworld\n");
        let size = iced::Size::new(300.0, 200.0);
        let mut renderer = renderer();

        let (cache, _) = frame(
            &mut editor,
            size,
            Cache::default(),
            &mut renderer,
            &[],
            mouse::Cursor::Unavailable,
        );

        // Well into the second row's vertical band, and far past the end of
        // "world" horizontally.
        let cursor = mouse::Cursor::Available(iced::Point::new(250.0, 45.0));
        let (cache, _) = frame(
            &mut editor,
            size,
            cache,
            &mut renderer,
            &[iced::Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left,
            ))],
            cursor,
        );
        let _ = frame(
            &mut editor,
            size,
            cache,
            &mut renderer,
            &[iced::Event::Mouse(mouse::Event::ButtonReleased(
                mouse::Button::Left,
            ))],
            cursor,
        );

        let at = editor.cursor();
        assert_eq!(
            editor.text_storage().line_of(at),
            1,
            "a click on the second row belongs on line 1 (\"world\")"
        );
        assert_eq!(
            at - editor.text_storage().line_start(1),
            "world".len(),
            "a click past the end of a short line clamps to that line's end"
        );
        assert!(
            editor.selection().is_empty(),
            "a press with no drag must not create a selection"
        );
    }

    #[test]
    fn dragging_selects_between_the_press_and_the_pointer() {
        let mut editor = editor("hello\nworld\n");
        let size = iced::Size::new(300.0, 200.0);
        let mut renderer = renderer();

        let (cache, _) = frame(
            &mut editor,
            size,
            Cache::default(),
            &mut renderer,
            &[],
            mouse::Cursor::Unavailable,
        );

        // Press at the very start of "hello" ...
        let (cache, _) = frame(
            &mut editor,
            size,
            cache,
            &mut renderer,
            &[iced::Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left,
            ))],
            mouse::Cursor::Available(iced::Point::new(2.0, 10.0)),
        );

        // ... drag past the end of "world" ...
        let end = iced::Point::new(250.0, 45.0);
        let cursor = mouse::Cursor::Available(end);
        let (cache, dragged) = frame(
            &mut editor,
            size,
            cache,
            &mut renderer,
            &[iced::Event::Mouse(mouse::Event::CursorMoved {
                position: end,
            })],
            cursor,
        );
        assert!(
            dragged.iter().any(|m| matches!(m, Interaction::Drag(_))),
            "every pointer movement during a drag must publish, or the host \
             never rebuilds the widget and the selection stops redrawing"
        );

        // ... and release.
        let _ = frame(
            &mut editor,
            size,
            cache,
            &mut renderer,
            &[iced::Event::Mouse(mouse::Event::ButtonReleased(
                mouse::Button::Left,
            ))],
            cursor,
        );

        let selection = editor.selection();
        assert_eq!(selection.anchor, 0, "the anchor stays at the press point");
        assert_eq!(
            selection.head,
            editor.text_storage().line_start(1) + "world".len(),
            "the head follows the pointer, clamped to the end of \"world\""
        );
        assert_eq!(
            editor.document().selected_text(),
            "hello\nworld",
            "the selection spans from the press to the pointer"
        );
    }

    /// The feature this refactor was for, end to end: select with the
    /// mouse, then press Backspace.
    #[test]
    fn a_dragged_selection_can_be_deleted() {
        let mut editor = editor("hello world");
        editor.perform(Action::Move(Motion::To(5)));
        editor.perform(Action::Select(Motion::DocumentEnd));
        assert_eq!(editor.selection(), Selection::new(5, 11));

        editor.perform(Action::Edit(Edit::Backspace));

        assert_eq!(editor.text(), "hello");
        assert_eq!(editor.selection(), Selection::caret(5));
    }

    /// Compares a render against a committed reference image.
    ///
    /// These catch vertical-rhythm, gutter and caret-placement drift
    /// without a display server. They are a *local* tripwire, not a
    /// specification: the references are generated on first run and are
    /// specific to this machine's renderer and fonts.
    fn assert_matches(
        ui: &mut iced_test::Simulator<'_, (), iced::Theme, iced::Renderer>,
        name: &str,
    ) {
        let snapshot = ui.snapshot(&iced::Theme::Light).expect("render");
        let path = format!("{}/tests/snapshots/{name}.png", env!("CARGO_MANIFEST_DIR"));
        assert!(
            snapshot.matches_image(path).expect("save snapshot"),
            "{name}: rendering drifted from the reference image"
        );
    }

    /// Narrow enough to force the first line to wrap, with the caret parked
    /// mid-document, so both the wrap path and the row/caret coordinate
    /// maths get exercised.
    #[test]
    fn snapshot_wrapped_caret() {
        let mut editor = editor(
            "The quick brown fox jumps over the lazy dog again and again and again\n\
             Second line here\nThird line for caret placement testing\n",
        );
        editor.perform(Action::Move(Motion::DocumentStart));
        editor.perform(Action::Move(Motion::Down));
        editor.perform(Action::Move(Motion::Down));
        for _ in 0..5 {
            editor.perform(Action::Move(Motion::Right));
        }

        let mut ui = simulator(&editor, iced::Size::new(360.0, 240.0));
        assert_matches(&mut ui, "wrapped_caret");
    }

    /// A selection running from partway into the first line to partway into
    /// the second should highlight to the end of the first and from the
    /// start of the second, exercising the per-row intersection maths.
    #[test]
    fn snapshot_selection() {
        let mut editor = editor("The quick brown fox\nSecond line here\nThird line\n");
        editor.document_mut().set_selection(Selection::new(10, 27));

        let mut ui = simulator(&editor, iced::Size::new(360.0, 160.0));
        assert_matches(&mut ui, "selection");
    }

    /// A document far taller than the viewport should render only the
    /// visible window, and the wheel should move that window.
    #[test]
    fn snapshot_scrolling() {
        let mut editor = editor(&lines(50));
        // Scroll-into-view follows the caret, which starts at the end of
        // the document; putting it back at the start is what makes this a
        // picture of the top.
        editor.perform(Action::Move(Motion::DocumentStart));

        let mut ui = simulator(&editor, iced::Size::new(200.0, 200.0));
        ui.point_at(iced_test::core::Point::new(50.0, 50.0));
        assert_matches(&mut ui, "scroll_top");

        for _ in 0..2 {
            ui.simulate([iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 },
            })]);
        }
        assert_matches(&mut ui, "scroll_down");
    }

    /// Hides markdown emphasis markers and bolds what they wrapped.
    struct Emphasis;

    impl<T: Text + ?Sized> Layer<T> for Emphasis {
        fn apply<'a>(&self, text: &'a T, row: &mut Row<'a>) {
            let start = text.line_start(row.line);
            let line = text.substring(start..text.line_end(row.line));
            let (Some(open), Some(close)) = (line.find("**"), line.rfind("**")) else {
                return;
            };
            if open >= close {
                return;
            }
            row.style(start + open + 2..start + close, |style| style.bold = true);
            row.conceal(start + close..start + close + 2);
            row.conceal(start + open..start + open + 2);
        }
    }

    /// Once a layer hides text, screen offsets and document positions stop
    /// agreeing, and every position the widget reports has to cross between
    /// them through the row's mapping.
    #[test]
    fn a_click_resolves_past_text_a_layer_hid() {
        let stack = (Plain, Emphasis);
        let mut editor = editor("a **bold** b");
        let size = iced::Size::new(300.0, 200.0);
        let mut renderer = renderer();

        let mut frame = |editor: &mut Editor<Rope>, cache, events: &[iced::Event], cursor| {
            let element: iced::Element<'_, Interaction, iced::Theme, iced::Renderer> = editor
                .view()
                .size(SIZE)
                .layer(&stack)
                .on_interaction(|interaction| interaction)
                .into();
            let mut interface = UserInterface::build(element, size, cache, &mut renderer);
            let mut messages = Vec::new();
            let _ = interface.update(
                &iced_test::core::window::Headless,
                &iced_test::core::shell::Waker::noop(),
                events,
                cursor,
                &mut renderer,
                &mut messages,
            );
            interface.draw(
                &mut renderer,
                &iced::Theme::Light,
                &Default::default(),
                cursor,
            );
            (interface.into_cache(), messages)
        };

        let cache = frame(
            &mut editor,
            Cache::default(),
            &[],
            mouse::Cursor::Unavailable,
        )
        .0;

        // Far past the right-hand end of the only line.
        let cursor = mouse::Cursor::Available(iced::Point::new(280.0, 10.0));
        let (_, messages) = frame(
            &mut editor,
            cache,
            &[iced::Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left,
            ))],
            cursor,
        );

        assert_eq!(
            messages.first(),
            Some(&Interaction::Press(12)),
            "the drawn row is 8 characters wide but stands for 12 document \
             bytes; a click at its end belongs at the end of the document"
        );
    }

    /// The same stack, rendered: the markers are gone and what they wrapped
    /// is bold.
    #[test]
    fn snapshot_layered() {
        let stack = (Plain, Emphasis);
        let editor = editor("a **bold** b\nplain line\n");

        let element: iced::Element<'_, (), iced::Theme, iced::Renderer> = editor
            .view()
            .size(SIZE)
            .show_line_numbers(true)
            .layer(&stack)
            .into();
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced_test::core::Size::new(360.0, 120.0),
            element,
        );
        assert_matches(&mut ui, "layered");
    }

    /// Touchpads send many small `Pixels` deltas per gesture, each far
    /// under one line. They have to accumulate: rounding each to the
    /// nearest line on its own would discard nearly every one.
    #[test]
    fn small_pixel_wheel_deltas_accumulate() {
        let editor = editor(&lines(50));
        let mut ui = simulator(&editor, iced::Size::new(200.0, 200.0));
        ui.point_at(iced_test::core::Point::new(50.0, 50.0));
        let _ = ui.snapshot(&iced::Theme::Light);

        let start = editor.viewport().scroll;
        // 20 events of 5px: 100px total, well over one line (~31px), but
        // each one is far under half a line on its own.
        for _ in 0..20 {
            ui.simulate([iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Pixels { x: 0.0, y: -5.0 },
            })]);
        }
        let _ = ui.snapshot(&iced::Theme::Light);

        assert_ne!(
            editor.viewport().scroll,
            start,
            "twenty 5px events must move the viewport"
        );
    }

    /// Every redraw builds a brand new widget, so "has the caret moved?"
    /// cannot be answered from anything the widget itself holds — it would
    /// reset each frame and read as a move on any relayout at all,
    /// snapping the viewport back onto a caret the user had scrolled away
    /// from. Only state kept in the widget tree survives to answer it,
    /// which is what these three frames exercise.
    #[test]
    fn an_unrelated_relayout_does_not_snap_back_to_the_caret() {
        let mut editor = editor("");
        editor.perform(Action::Edit(Edit::Paste(lines(50))));

        let size = iced::Size::new(200.0, 200.0);
        let mut renderer = renderer();
        let nowhere = mouse::Cursor::Unavailable;
        let over_editor = mouse::Cursor::Available(iced::Point::new(50.0, 50.0));

        // Frame 1: the first layout follows the caret to the end.
        let (cache, _) = frame(
            &mut editor,
            size,
            Cache::default(),
            &mut renderer,
            &[],
            nowhere,
        );
        assert!(
            editor.viewport().scroll.0 > 0,
            "the first layout should have followed the caret down"
        );

        // Frame 2: the user wheels all the way back to the top.
        let (cache, _) = frame(
            &mut editor,
            size,
            cache,
            &mut renderer,
            &[iced::Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { x: 0.0, y: 1000.0 },
            })],
            over_editor,
        );
        assert_eq!(editor.viewport().scroll, (0, 0.0));

        // Frame 3: a rebuild with no wheel event and no caret movement.
        let _ = frame(&mut editor, size, cache, &mut renderer, &[], nowhere);

        assert_eq!(
            editor.viewport().scroll,
            (0, 0.0),
            "an unrelated relayout must leave the viewport where the user put it"
        );
    }

    #[test]
    fn moving_the_caret_below_the_window_scrolls_to_follow() {
        let mut editor = editor(&lines(50));
        editor.perform(Action::Move(Motion::DocumentStart));

        {
            let mut ui = simulator(&editor, iced::Size::new(200.0, 200.0));
            let _ = ui.snapshot(&iced::Theme::Light);
        }
        assert_eq!(editor.viewport().scroll.0, 0, "starts at the top");

        editor.perform(Action::Move(Motion::DocumentEnd));
        let cursor_line = editor.text_storage().line_of(editor.cursor());

        let mut ui = simulator(&editor, iced::Size::new(200.0, 200.0));
        let _ = ui.snapshot(&iced::Theme::Light);

        let (anchor, _) = editor.viewport().scroll;
        assert!(anchor > 0, "the viewport should have scrolled down");
        assert!(
            cursor_line >= anchor,
            "the caret's line {cursor_line} must be at or below the anchor {anchor}"
        );
    }

    #[test]
    fn moving_the_caret_above_the_window_scrolls_back_up() {
        let mut editor = editor(&lines(50));

        {
            let mut ui = simulator(&editor, iced::Size::new(200.0, 200.0));
            ui.point_at(iced_test::core::Point::new(50.0, 50.0));
            let _ = ui.snapshot(&iced::Theme::Light);
            ui.simulate([iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y: -5.0 },
            })]);
            let _ = ui.snapshot(&iced::Theme::Light);
        }
        assert!(editor.viewport().scroll.0 > 0, "scrolled down first");

        editor.perform(Action::Move(Motion::DocumentStart));

        let mut ui = simulator(&editor, iced::Size::new(200.0, 200.0));
        let _ = ui.snapshot(&iced::Theme::Light);

        assert_eq!(
            editor.viewport().scroll,
            (0, 0.0),
            "the viewport should have revealed the caret at the start"
        );
    }

    /// Following the caret has to be measured in pixels, not in line
    /// counts: a line that wraps into more rows pushes its own tail below
    /// the viewport without the count of lines on screen changing at all.
    #[test]
    fn a_line_that_wraps_past_the_bottom_scrolls_to_follow() {
        let mut editor = editor("");
        editor.perform(Action::Edit(Edit::Paste(format!(
            "short\n{}",
            "wrap ".repeat(20)
        ))));

        let mut ui = simulator(&editor, iced::Size::new(150.0, 80.0));
        let _ = ui.snapshot(&iced::Theme::Light);

        assert_ne!(
            editor.viewport().scroll,
            (0, 0.0),
            "the viewport should have followed the caret onto the wrapped \
             line's overflowing tail"
        );
    }

    /// The caret's line must be brought *fully* into view. "The right line
    /// is somewhere in the window" is not enough: a wheel nudge smaller
    /// than one line leaves it straddling the top edge, and half a line of
    /// caret is still a caret you cannot see.
    #[test]
    fn a_partially_cut_off_caret_line_snaps_flush_to_the_top() {
        let mut editor = editor(&lines(10));
        editor.perform(Action::Move(Motion::DocumentStart));

        {
            let mut ui = simulator(&editor, iced::Size::new(200.0, 200.0));
            ui.point_at(iced_test::core::Point::new(50.0, 50.0));
            let _ = ui.snapshot(&iced::Theme::Light);
            // Well under one line's height, so the anchor stays on line 0
            // but scrolls partway into it. The wheel's own relayout
            // deliberately suppresses scroll-into-view, so this state
            // survives that pass.
            ui.simulate([iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Pixels { x: 0.0, y: -10.0 },
            })]);
        }
        let (line, offset) = editor.viewport().scroll;
        assert_eq!(line, 0);
        assert!(offset > 0.0, "the nudge should have scrolled into line 0");

        // A fresh layout pass, not wheel-driven, with the caret on line 0.
        let mut ui = simulator(&editor, iced::Size::new(200.0, 200.0));
        let _ = ui.snapshot(&iced::Theme::Light);

        assert_eq!(
            editor.viewport().scroll,
            (0, 0.0),
            "the caret's line was cut off at the top; the viewport should have \
             snapped flush to it"
        );
    }
}
