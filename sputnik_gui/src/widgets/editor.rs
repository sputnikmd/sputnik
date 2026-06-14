use std::marker::PhantomData;
use std::sync::Arc;

use iced::Element;
use iced::widget::text::Span;
use iced::widget::{column, text};

use crate::widgets::EditorParagraph;

pub enum Action {
    MoveCursorLeft,
    MoveCursorRight,
}

pub struct Editor<'a, Message> {
    buffer: Arc<String>,
    span_cache: Vec<Span<'a, (), iced::Font>>,
    cursor: usize,
    _phantom_data: PhantomData<Message>,
}

impl<'a, Message: 'a> Editor<'a, Message> {
    pub fn new(buffer: Arc<String>) -> Editor<'a, Message> {
        let span = Span::new((*buffer).clone());

        Self {
            buffer,
            span_cache: vec![span],
            cursor: 0,
            _phantom_data: PhantomData,
        }
    }

    pub fn action(&mut self, action: Action) {
        match action {
            Action::MoveCursorLeft => {
                self.cursor = self.cursor.saturating_sub(1);
            }
            Action::MoveCursorRight => {
                let max = self.buffer.len();
                self.cursor = (self.cursor + 1).min(max);
            }
        }
    }

    pub fn to_element<'b>(&'b self) -> Element<'b, Message>
    where
        'a: 'b,
    {
        let total: usize = self.buffer.len();

        let text_widget =
            EditorParagraph::with_spans(self.span_cache.as_slice(), Some(self.cursor))
                .size(24.0)
                .cursor_color(iced::color!(0x000000));

        let hud = text(format!("cursor: {}/{}", self.cursor, total))
            .size(14.0)
            .color(iced::color!(0x666666));

        column([text_widget.into(), hud.into()]).spacing(8).into()
    }
}
