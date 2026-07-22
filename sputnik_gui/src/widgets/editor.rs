use std::cell::Cell;

use ropey::Rope;

use crate::widgets::EditorParagraph;

const FONT_SIZE: f32 = 24.0;

pub enum Action {
    MoveCursorLeft,
    MoveCursorRight,
    MoveCursorUp,
    MoveCursorDown,
    Insert(char),
    InsertTab,
    DeleteBackward,
    DeleteForward,
}

pub struct Editor<Message> {
    buffer: Rope,
    cursor: usize,
    tab_size: usize,
    /// Wrapping width for visual navigation. Interior mutability so callers
    /// with only `&self` can update it after layout.
    nav_width: Cell<f32>,
    /// Topmost visible logical line. Interior mutability for the same
    /// reason as `nav_width`: the widget updates it on scroll, `Editor`
    /// reads it for keyboard-driven navigation.
    scroll_anchor: Cell<usize>,
    _phantom: std::marker::PhantomData<Message>,
}

impl<Message> Editor<Message> {
    pub fn new(buffer: Rope) -> Editor<Message> {
        Editor {
            buffer,
            cursor: 0,
            tab_size: 4,
            nav_width: Cell::new(800.0),
            scroll_anchor: Cell::new(0),
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn action(&mut self, action: Action) {
        match action {
            Action::MoveCursorLeft => {
                self.cursor = self.cursor.saturating_sub(1);
            }
            Action::MoveCursorRight => {
                self.cursor = (self.cursor + 1).min(self.buffer.len_chars());
            }
            Action::MoveCursorUp => {
                self.cursor = self.navigate_visual(-1);
            }
            Action::MoveCursorDown => {
                self.cursor = self.navigate_visual(1);
            }
            Action::Insert(ch) => {
                let idx = self.cursor;
                self.edit(|rope| rope.insert(idx, ch.encode_utf8(&mut [0; 4])));
                self.cursor += 1;
            }
            Action::DeleteBackward => {
                if self.cursor > 0 {
                    let idx = self.cursor;
                    self.edit(|rope| rope.remove((idx - 1)..idx));
                    self.cursor = idx - 1;
                }
            }
            Action::DeleteForward => {
                if self.cursor < self.buffer.len_chars() {
                    let idx = self.cursor;
                    self.edit(|rope| rope.remove(idx..(idx + 1)));
                }
            }
            Action::InsertTab => {
                let idx = self.cursor;
                let spaces = " ".repeat(self.tab_size);
                self.edit(|rope| rope.insert(idx, &spaces));
                self.cursor += self.tab_size;
            }
        }
    }

    fn flat_cursor(&self) -> usize {
        self.buffer.char_to_byte(self.cursor)
    }

    fn navigate_visual(&self, direction: isize) -> usize {
        let width = self.nav_width.get();
        let flat_cursor = self.flat_cursor();

        // Determine which logical lines we need to shape for navigation.
        let cursor_line = self.buffer.char_to_line(self.cursor);
        let n_lines = self.buffer.len_lines();

        let (seg_start, seg_end) = if direction < 0 {
            let start = if cursor_line > 0 {
                cursor_line - 1
            } else {
                cursor_line
            };
            (start, cursor_line + 1)
        } else {
            let end = (cursor_line + 2).min(n_lines);
            (cursor_line, end)
        };

        let seg_char_start = self.buffer.line_to_char(seg_start);
        let seg_char_end = if seg_end < n_lines {
            self.buffer.line_to_char(seg_end)
        } else {
            self.buffer.len_chars()
        };

        let seg_slice = self.buffer.slice(seg_char_start..seg_char_end);
        let seg_bytes_before = self.buffer.char_to_byte(seg_char_start);

        if seg_slice.len_bytes() == 0 {
            return self.cursor;
        }

        // Flatten the segment into a contiguous string (only the relevant lines).
        let seg_str = seg_slice.to_string();

        // Shape the segment to find visual lines, cursor X, and closest glyph.
        let fs = iced::advanced::graphics::text::font_system();
        let mut font_system = fs.write().expect("font system lock");

        let metrics = cosmic_text::Metrics::new(FONT_SIZE, 30.0);
        let mut buf = cosmic_text::Buffer::new(font_system.raw(), metrics);
        buf.set_size(Some(width), None);
        buf.set_text(
            &seg_str,
            &cosmic_text::Attrs::new(),
            cosmic_text::Shaping::Advanced,
            None,
        );
        buf.shape_until_scroll(font_system.raw(), false);
        drop(font_system);

        // Build line_starts for logical lines within the segment.
        let mut line_starts = Vec::with_capacity(buf.lines.len());
        let mut off = 0usize;
        for li in 0..buf.lines.len() {
            line_starts.push(off);
            off += buf.lines[li].text().len();
            if li + 1 < buf.lines.len() {
                off += 1;
            }
        }

        // Collect visual lines (layout runs) with their global byte ranges.
        struct VLine {
            local_start: usize,
            local_end: usize,
            run_index: usize,
            line_i: usize,
        }

        let layout_runs: Vec<_> = buf.layout_runs().collect();
        let mut vlines: Vec<VLine> = Vec::with_capacity(layout_runs.len());

        for (ri, run) in layout_runs.iter().enumerate() {
            let base = line_starts.get(run.line_i).copied().unwrap_or(0);
            let (mn, mx) = if run.glyphs.is_empty() {
                (0, 0)
            } else {
                let mn = run.glyphs.iter().map(|g| g.start).min().unwrap();
                let mx = run.glyphs.iter().map(|g| g.end).max().unwrap();
                (mn, mx)
            };
            vlines.push(VLine {
                local_start: base + mn,
                local_end: base + mx,
                run_index: ri,
                line_i: run.line_i,
            });
        }
        vlines.sort_by_key(|vl| vl.local_start);

        if vlines.is_empty() {
            return self.cursor;
        }

        let seg_flat_cursor = flat_cursor.saturating_sub(seg_bytes_before);

        // Find which visual line the cursor is on.
        let cur_i = vlines
            .iter()
            .position(|vl| seg_flat_cursor >= vl.local_start && seg_flat_cursor <= vl.local_end)
            .or_else(|| {
                vlines
                    .iter()
                    .rposition(|vl| vl.local_start <= seg_flat_cursor)
            })
            .unwrap_or(0);

        let target_i = (cur_i as isize + direction).clamp(0, vlines.len() as isize - 1) as usize;
        if target_i == cur_i {
            return if direction < 0 {
                0
            } else {
                self.buffer.len_chars()
            };
        }

        // Get cursor X position on the current visual line.
        let cur_vl = &vlines[cur_i];
        let base = line_starts.get(cur_vl.line_i).copied().unwrap_or(0);
        let local = seg_flat_cursor.saturating_sub(base);
        let cosmic_cursor = cosmic_text::Cursor::new(cur_vl.line_i, local);
        let cur_x = layout_runs[cur_vl.run_index]
            .cursor_position(&cosmic_cursor)
            .unwrap_or(0.0);

        // Find the glyph closest to cur_x on the target visual line.
        let tgt_vl = &vlines[target_i];
        let tgt_run = &layout_runs[tgt_vl.run_index];

        let mut best_local = tgt_vl.local_start;
        let mut best_dist = f32::MAX;
        for g in tgt_run.glyphs {
            let start_local = tgt_vl.local_start + g.start;
            let end_local = (tgt_vl.local_start + g.end).min(tgt_vl.local_end);
            for (x, local) in [(g.x, start_local), (g.x + g.w, end_local)] {
                let d = (x - cur_x).abs();
                if d < best_dist {
                    best_dist = d;
                    best_local = local;
                }
            }
        }

        let global_byte = seg_bytes_before + best_local;
        self.buffer
            .byte_to_char(global_byte.min(self.buffer.len_bytes()))
    }

    pub fn edit(&mut self, f: impl FnOnce(&mut Rope)) {
        f(&mut self.buffer);
        self.cursor = self.cursor.min(self.buffer.len_chars());
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn total_chars(&self) -> usize {
        self.buffer.len_chars()
    }

    pub fn view(&self) -> EditorParagraph<'_, iced::Theme, iced::Renderer> {
        EditorParagraph::new(
            &self.buffer,
            Some(self.flat_cursor()),
            &self.nav_width,
            &self.scroll_anchor,
        )
        .size(FONT_SIZE)
        .show_line_numbers(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_delete() {
        let mut editor = Editor::<()>::new(Rope::from_str(""));
        editor.action(Action::Insert('a'));
        assert_eq!(editor.cursor(), 1);
        assert_eq!(editor.buffer.to_string(), "a");
        editor.action(Action::Insert('\n'));
        assert_eq!(editor.cursor(), 2);
        assert_eq!(editor.buffer.to_string(), "a\n");
        editor.action(Action::DeleteBackward);
        assert_eq!(editor.cursor(), 1);
        assert_eq!(editor.buffer.to_string(), "a");
    }

    /// Headless render regression test: catches vertical-rhythm, gutter and
    /// cursor-placement bugs in the per-line row cache (see `EditorParagraph`)
    /// without needing a real window or display server.
    ///
    /// Narrow enough to force the first row to wrap, with the cursor moved
    /// into the middle of the document, so both the wrap path and the
    /// row/cursor coordinate math get exercised.
    #[test]
    fn snapshot_wrapped_cursor() {
        let mut editor = Editor::<()>::new(Rope::from_str(""));
        let text = "The quick brown fox jumps over the lazy dog again and again and again\nSecond line here\nThird line for cursor placement testing\n";
        for ch in text.chars() {
            editor.action(Action::Insert(ch));
        }
        for _ in 0..text.chars().count() {
            editor.action(Action::MoveCursorLeft);
        }
        editor.action(Action::MoveCursorDown);
        editor.action(Action::MoveCursorDown);
        for _ in 0..5 {
            editor.action(Action::MoveCursorRight);
        }

        let element: iced::Element<'_, (), iced::Theme, iced::Renderer> = editor.view().into();
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced_test::core::Size::new(360.0, 240.0),
            element,
        );
        let snapshot = ui
            .snapshot(&iced::Theme::Light)
            .expect("snapshot should render");
        let matches = snapshot
            .matches_image(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/snapshots/editor_wrapped_cursor.png"
            ))
            .expect("snapshot should save");
        assert!(
            matches,
            "rendering drifted from the committed reference image"
        );
    }

    /// Headless regression test for viewport virtualization: a document
    /// much taller than the viewport should only render the visible window,
    /// and wheel scrolling should move that window by whole lines.
    #[test]
    fn snapshot_scrolling() {
        let mut editor = Editor::<()>::new(Rope::from_str(""));
        let mut text = String::new();
        for i in 1..=50 {
            text.push_str(&format!("line {i}\n"));
        }
        for ch in text.chars() {
            editor.action(Action::Insert(ch));
        }

        let element: iced::Element<'_, (), iced::Theme, iced::Renderer> = editor.view().into();
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced_test::core::Size::new(200.0, 200.0),
            element,
        );
        ui.point_at(iced_test::core::Point::new(50.0, 50.0));

        let top = ui
            .snapshot(&iced::Theme::Light)
            .expect("snapshot should render");
        let top_matches = top
            .matches_image(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/snapshots/editor_scroll_top.png"
            ))
            .expect("snapshot should save");
        assert!(top_matches, "top-of-document rendering drifted");

        // Two ticks of -1 line each; the sign convention matches
        // `iced::widget::scrollable` (raw delta negated), and the widget
        // multiplies by 3, so this moves the anchor down by 6 lines.
        for _ in 0..2 {
            ui.simulate([iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 },
            })]);
        }

        let scrolled = ui
            .snapshot(&iced::Theme::Light)
            .expect("snapshot should render");
        let scrolled_matches = scrolled
            .matches_image(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/snapshots/editor_scroll_down.png"
            ))
            .expect("snapshot should save");
        assert!(scrolled_matches, "scrolled rendering drifted");
    }
}
