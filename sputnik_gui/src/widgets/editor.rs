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
    char_count: usize,
    _phantom_data: PhantomData<Message>,
}

impl<'a, Message: 'a> Editor<'a, Message> {
    pub fn new(buffer: Arc<String>) -> Editor<'a, Message> {
        let span = Span::new((*buffer).clone());
        let char_count = buffer.chars().count();

        Self {
            buffer,
            span_cache: vec![span],
            cursor: 0,
            char_count,
            _phantom_data: PhantomData,
        }
    }

    pub fn action(&mut self, action: Action) {
        match action {
            Action::MoveCursorLeft => {
                self.cursor = self.cursor.saturating_sub(1);
            }
            Action::MoveCursorRight => {
                self.cursor = (self.cursor + 1).min(self.char_count);
            }
        }
    }

    fn cursor_byte_offset(&self) -> usize {
        self.buffer
            .char_indices()
            .nth(self.cursor)
            .map(|(offset, _)| offset)
            .unwrap_or(self.buffer.len())
    }

    pub fn to_element<'b>(&'b self) -> Element<'b, Message>
    where
        'a: 'b,
    {
        let byte_cursor = self.cursor_byte_offset();

        let text_widget =
            EditorParagraph::with_spans(self.span_cache.as_slice(), Some(byte_cursor))
                .size(24.0)
                .cursor_color(iced::color!(0x000000));

        let hud = text(format!("cursor: {}/{}", self.cursor, self.char_count))
            .size(14.0)
            .color(iced::color!(0x666666));

        column([text_widget.into(), hud.into()]).spacing(8).into()
    }
}
