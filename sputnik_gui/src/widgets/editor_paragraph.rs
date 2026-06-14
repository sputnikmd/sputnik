use std::any::Any;

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
    cursor_color: Color,
    cursor_width: f32,
    class: Theme::Class<'a>,
}

struct State<Link, P: Paragraph> {
    spans: Vec<Span<'static, Link, P::Font>>,
    paragraph: P,
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
    ) -> Self {
        Self {
            spans: Box::new(spans),
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
            cursor_color: Color::BLACK,
            cursor_width: 2.0,
            class: Theme::default(),
        }
    }

    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = Some(size.into());
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    pub fn cursor_color(mut self, color: impl Into<Color>) -> Self {
        self.cursor_color = color.into();
        self
    }

    pub fn cursor_width(mut self, width: f32) -> Self {
        self.cursor_width = width;
        self
    }
}

fn spans_total_len<Link, Font>(spans: &[Span<'_, Link, Font>]) -> usize {
    spans.iter().map(|s| s.text.len()).sum()
}

impl<Link, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for EditorParagraph<'_, Link, Theme, Renderer>
where
    Link: Clone + 'static,
    Renderer: text_advanced::Renderer + 'static,
    Theme: Catalog,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State<Link, Renderer::Paragraph>>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::<Link, _> {
            spans: Vec::new(),
            paragraph: Renderer::Paragraph::default(),
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
        let state = tree
            .state
            .downcast_mut::<State<Link, Renderer::Paragraph>>();

        layout::sized(limits, self.width, self.height, |limits| {
            let bounds = limits.max();
            let size = self.size.unwrap_or_else(|| renderer.default_size());
            let font = self.font.unwrap_or_else(|| renderer.default_font());

            let text_with_spans = || text_advanced::Text {
                content: self.spans.as_ref().as_ref(),
                bounds,
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

            if state.spans != self.spans.as_ref().as_ref() {
                state.paragraph = Renderer::Paragraph::with_spans(text_with_spans());
                state.spans = self
                    .spans
                    .as_ref()
                    .as_ref()
                    .iter()
                    .cloned()
                    .map(Span::to_static)
                    .collect();
            } else {
                match state.paragraph.compare(text_advanced::Text {
                    content: (),
                    bounds,
                    size,
                    line_height: self.line_height,
                    font,
                    align_x: self.align_x,
                    align_y: self.align_y,
                    shaping: Shaping::Advanced,
                    wrapping: self.wrapping,
                    ellipsis: self.ellipsis,
                    hint_factor: renderer.scale_factor(),
                }) {
                    text_advanced::Difference::None => {}
                    text_advanced::Difference::Bounds => {
                        state.paragraph.resize(bounds);
                    }
                    text_advanced::Difference::Shape => {
                        state.paragraph = Renderer::Paragraph::with_spans(text_with_spans());
                    }
                }
            }

            state.paragraph.min_bounds()
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
        let state = tree
            .state
            .downcast_ref::<State<Link, Renderer::Paragraph>>();
        let appearance = theme.style(&self.class);
        let bounds = layout.bounds();

        let anchor = bounds.anchor(
            state.paragraph.min_bounds(),
            state.paragraph.align_x(),
            state.paragraph.align_y(),
        );

        let color = appearance.color.unwrap_or(style.text_color);

        renderer.fill_paragraph(&state.paragraph, anchor, color, *viewport);

        let total = spans_total_len(self.spans.as_ref().as_ref());
        if let Some(cursor) = self.cursor
            && cursor <= total
        {
            draw_cursor(
                renderer,
                &state.paragraph,
                anchor,
                cursor,
                self.cursor_color,
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
    paragraph: &Renderer::Paragraph,
    anchor: Point,
    cursor: usize,
    color: Color,
    width: f32,
) {
    let hint_factor = Paragraph::hint_factor(paragraph).unwrap_or(1.0);

    let any: &dyn Any = paragraph as &dyn Any;
    let Some(graphics_paragraph) = any.downcast_ref::<GraphicsParagraph>() else {
        return;
    };

    let buffer = graphics_paragraph.buffer();
    let Some((cx, cy, ch)) = cursor_position(buffer, cursor, hint_factor) else {
        return;
    };

    renderer.fill_quad(
        Quad {
            bounds: Rectangle {
                x: anchor.x + cx,
                y: anchor.y + cy,
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

fn cursor_position(
    buffer: &cosmic_text::Buffer,
    cursor: usize,
    hint_factor: f32,
) -> Option<(f32, f32, f32)> {
    let mut line_start = 0usize;

    for line_i in 0..buffer.lines.len() {
        let line_text = buffer.lines[line_i].text();
        let line_end = line_start + line_text.len();

        if cursor > line_end && line_i + 1 < buffer.lines.len() {
            line_start = line_end + 1;
            continue;
        }

        let local_cursor = if cursor <= line_end {
            cursor - line_start
        } else {
            line_text.len()
        };

        let cosmic_cursor = cosmic_text::Cursor::new(line_i, local_cursor);

        for run in buffer.layout_runs() {
            if run.line_i != line_i {
                continue;
            }

            if let Some(x) = run.cursor_position(&cosmic_cursor) {
                return Some((
                    x / hint_factor,
                    run.line_top / hint_factor,
                    (run.line_height + 1.0) / hint_factor,
                ));
            }
        }

        line_start = line_end + 1;
    }

    let metrics = buffer.metrics();
    Some((0.0, 0.0, (metrics.line_height + 1.0) / hint_factor))
}
