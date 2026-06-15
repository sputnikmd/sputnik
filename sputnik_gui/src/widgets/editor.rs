use std::marker::PhantomData;

use iced::Element;
use iced::widget::text::Span;
use ropey::Rope;

use crate::widgets::EditorParagraph;

pub enum Action {
    MoveCursorLeft,
    MoveCursorRight,
    Insert(char),
    InsertTab,
    DeleteBackward,
    DeleteForward,
}

pub struct Editor<Message> {
    buffer: Rope,
    cursor: usize,
    cached_flat: Vec<Span<'static, (), iced::Font>>,
    #[allow(dead_code)]
    cached_lines: Vec<Vec<Span<'static, (), iced::Font>>>,
    _buffer_revision: usize,
    tab_size: usize,
    _phantom_data: PhantomData<Message>,
}

const INITIAL_REVISION: usize = 1;

impl<Message> Editor<Message> {
    pub fn new(buffer: Rope) -> Editor<Message> {
        let (cached_flat, cached_lines) = build_cache(&buffer);

        Self {
            buffer,
            cursor: 0,
            cached_flat,
            cached_lines,
            _buffer_revision: INITIAL_REVISION,
            tab_size: 4,
            _phantom_data: PhantomData,
        }
    }

    pub fn action(&mut self, action: Action) {
        match action {
            Action::MoveCursorLeft => {
                self.cursor = self.cursor.saturating_sub(1);
            }
            Action::MoveCursorRight => {
                let max = self.buffer.len_chars();
                self.cursor = (self.cursor + 1).min(max);
            }
            Action::Insert(ch) => {
                let byte_pos = self.buffer.char_to_byte(self.cursor);
                self.edit(|rope| rope.insert(byte_pos, ch.encode_utf8(&mut [0; 4])));
                self.cursor += 1;
            }
            Action::DeleteBackward => {
                if self.cursor > 0 {
                    let byte_start = self.buffer.char_to_byte(self.cursor - 1);
                    let byte_end = self.buffer.char_to_byte(self.cursor);
                    let idx = self.cursor;
                    self.edit(|rope| rope.remove(byte_start..byte_end));
                    self.cursor = idx - 1;
                }
            }
            Action::DeleteForward => {
                if self.cursor < self.buffer.len_chars() {
                    let byte_start = self.buffer.char_to_byte(self.cursor);
                    let byte_end = self.buffer.char_to_byte(self.cursor + 1);
                    self.edit(|rope| rope.remove(byte_start..byte_end));
                }
            }
            Action::InsertTab => {
                let spaces: String = " ".repeat(self.tab_size);
                let byte_pos = self.buffer.char_to_byte(self.cursor);
                self.edit(|rope| rope.insert(byte_pos, spaces.as_str()));
                self.cursor += self.tab_size;
            }
        }
    }

    pub fn edit(&mut self, f: impl FnOnce(&mut Rope)) {
        f(&mut self.buffer);
        self._buffer_revision += 1;
        (self.cached_flat, self.cached_lines) = build_cache(&self.buffer);
        self.cursor = self.cursor.min(self.buffer.len_chars());
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn cursor_byte_offset(&self) -> usize {
        self.buffer.char_to_byte(self.cursor)
    }

    pub fn buffer(&self) -> &Rope {
        &self.buffer
    }

    pub fn total_chars(&self) -> usize {
        self.buffer.len_chars()
    }

    pub fn tab_size(&self) -> usize {
        self.tab_size
    }

    pub fn set_tab_size(&mut self, size: usize) {
        self.tab_size = size;
    }

    pub fn view<'b>(&'b self) -> Element<'b, Message> {
        let byte_cursor = self.cursor_byte_offset();

        EditorParagraph::with_spans(self.cached_flat.as_slice(), Some(byte_cursor))
            .size(24.0)
            .cursor_color(iced::color!(0x000000))
            .into()
    }
}

fn build_cache(
    buffer: &Rope,
) -> (
    Vec<Span<'static, (), iced::Font>>,
    Vec<Vec<Span<'static, (), iced::Font>>>,
) {
    let mut flat = Vec::new();
    let mut lines = Vec::new();

    for line_slice in buffer.lines() {
        let span = Span::new(line_slice.to_string());
        lines.push(vec![span.clone()]);
        flat.push(span);
    }

    (flat, lines)
}
