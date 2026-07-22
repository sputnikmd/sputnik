use std::cell::Cell;
use std::ops::Range;

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

/// Normalizes `start`/`end` into ascending order and clips both to `len`.
/// Returns `None` when that leaves nothing selected (an empty or fully
/// out-of-bounds range), rather than a vacuous `Some(x..x)`.
fn clamp_selection(start: usize, end: usize, len: usize) -> Option<Range<usize>> {
    let (start, end) = (start.min(len), end.min(len));
    let (start, end) = (start.min(end), start.max(end));
    (start < end).then_some(start..end)
}

pub struct Editor<Message> {
    buffer: Rope,
    cursor: usize,
    tab_size: usize,
    /// Wrapping width for visual navigation. Interior mutability so callers
    /// with only `&self` can update it after layout.
    nav_width: Cell<f32>,
    /// Topmost visible logical line, plus how many pixels of that line are
    /// scrolled above the viewport. Interior mutability for the same reason
    /// as `nav_width`: the widget updates it on scroll.
    scroll_anchor: Cell<(usize, f32)>,
    /// Selected char range, `start..end` with `start <= end`. `None` means
    /// no selection.
    selection: Option<Range<usize>>,
    _phantom: std::marker::PhantomData<Message>,
}

impl<Message> Editor<Message> {
    pub fn new(buffer: Rope) -> Editor<Message> {
        Editor {
            buffer,
            cursor: 0,
            tab_size: 4,
            nav_width: Cell::new(800.0),
            scroll_anchor: Cell::new((0, 0.0)),
            selection: None,
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
        let tgt_line_base = line_starts.get(tgt_vl.line_i).copied().unwrap_or(0);
        for g in tgt_run.glyphs {
            // g.start / g.end are byte offsets within the *logical* line,
            // not the visual line. When word-wrapping creates several
            // visual lines out of one logical line, tgt_vl.local_start
            // already carries the visual-line offset — adding g.start
            // on top of it would double-count, producing a position far
            // outside the segment. Use the logical-line base instead.
            let start_local = tgt_line_base + g.start;
            let end_local = (tgt_line_base + g.end).min(tgt_vl.local_end);
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
        let len = self.buffer.len_chars();
        self.cursor = self.cursor.min(len);
        // An edit can shrink the buffer out from under a selection set
        // before it (e.g. deleting past what was selected): re-clamp rather
        // than let it go stale and panic on the next `char_to_byte`.
        self.selection = self
            .selection
            .take()
            .and_then(|range| clamp_selection(range.start, range.end, len));
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn total_chars(&self) -> usize {
        self.buffer.len_chars()
    }

    /// Sets the selection to the char range between `a` and `b`, in either
    /// order. Out-of-bounds endpoints are clipped to the buffer's length;
    /// if that clipping collapses the range to empty, the selection is
    /// cleared instead of kept as a vacuous `Some(x..x)`.
    ///
    /// Not yet called from `Application` — no selection gesture (mouse drag,
    /// shift+arrow) is wired up yet, only the widget-level rendering and
    /// this API to drive it.
    #[allow(dead_code)]
    pub fn select(&mut self, a: usize, b: usize) {
        self.selection = clamp_selection(a, b, self.buffer.len_chars());
    }

    #[allow(dead_code)]
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    #[allow(dead_code)]
    pub fn selection(&self) -> Option<Range<usize>> {
        self.selection.clone()
    }

    /// The current selection as a byte range, for the widget layer (which
    /// otherwise deals only in byte offsets into the rope).
    ///
    /// Re-validates against the current buffer length even though `select`
    /// and `edit` already keep `self.selection` in bounds: this is the last
    /// line of defense before the only place that can actually panic
    /// (`char_to_byte` on an out-of-bounds char index), so it stays correct
    /// even if some future caller mutates `self.selection` directly.
    fn flat_selection(&self) -> Option<Range<usize>> {
        let selection = self.selection.as_ref()?;
        let len = self.buffer.len_chars();
        let range = clamp_selection(selection.start, selection.end, len)?;
        Some(self.buffer.char_to_byte(range.start)..self.buffer.char_to_byte(range.end))
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
        .selection(self.flat_selection())
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

    #[test]
    fn select_normalizes_order_and_clamps_to_buffer_len() {
        let mut editor = Editor::<()>::new(Rope::from_str("hello world"));

        editor.select(2, 5);
        assert_eq!(editor.selection(), Some(2..5));

        // Reversed endpoints (selecting backwards) still normalize to a
        // forward range.
        editor.select(5, 2);
        assert_eq!(editor.selection(), Some(2..5));

        // Out-of-bounds endpoints clamp to the buffer's length.
        editor.select(8, 1000);
        assert_eq!(editor.selection(), Some(8..11));

        editor.clear_selection();
        assert_eq!(editor.selection(), None);
    }

    #[test]
    fn select_on_empty_buffer_does_not_panic() {
        let mut editor = Editor::<()>::new(Rope::from_str(""));
        editor.select(5, 10);
        assert_eq!(editor.selection(), None);
        assert_eq!(editor.flat_selection(), None);
    }

    #[test]
    fn selection_partially_out_of_bounds_after_shrink_is_clamped() {
        let mut editor = Editor::<()>::new(Rope::from_str("hello world"));
        editor.select(2, 8);

        // Shrink so only the selection's start remains valid.
        editor.edit(|rope| rope.remove(4..rope.len_chars()));

        assert_eq!(editor.selection(), Some(2..4));
    }

    /// Regression test for a panic report: selecting a range and then
    /// deleting the buffer out from under it left a stale, out-of-bounds
    /// selection that `char_to_byte` panicked on the next time `view()`
    /// rendered it.
    #[test]
    fn selection_survives_edits_that_shrink_the_buffer_without_panicking() {
        let mut editor = Editor::<()>::new(Rope::from_str("hello world"));
        editor.select(2, 8);
        assert_eq!(editor.selection(), Some(2..8));

        editor.edit(|rope| rope.remove(0..rope.len_chars()));

        assert_eq!(editor.selection(), None);
        assert_eq!(editor.flat_selection(), None);

        let element: iced::Element<'_, (), iced::Theme, iced::Renderer> = editor.view().into();
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced_test::core::Size::new(200.0, 200.0),
            element,
        );
        let _ = ui.snapshot(&iced::Theme::Light);
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

    /// Headless render regression test for the selection layer: a selection
    /// spanning from partway into the first line to partway into the second
    /// should highlight to the end of the first line and from the start of
    /// the second, exercising the per-row intersection math in
    /// `draw_selection`.
    #[test]
    fn snapshot_selection() {
        let mut editor = Editor::<()>::new(Rope::from_str(""));
        let text = "The quick brown fox\nSecond line here\nThird line\n";
        for ch in text.chars() {
            editor.action(Action::Insert(ch));
        }
        editor.select(10, 27);

        let element: iced::Element<'_, (), iced::Theme, iced::Renderer> = editor.view().into();
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced_test::core::Size::new(360.0, 160.0),
            element,
        );
        let snapshot = ui
            .snapshot(&iced::Theme::Light)
            .expect("snapshot should render");
        let matches = snapshot
            .matches_image(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/snapshots/editor_selection.png"
            ))
            .expect("snapshot should save");
        assert!(
            matches,
            "selection rendering drifted from the committed reference image"
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

    /// Regression test: navigating up through an empty line should not
    /// cause the cursor to go back down after a couple of moves.
    #[test]
    fn navigate_visual_through_empty_line_does_not_oscillate() {
        let text = "zero\none\n\nthree\n\nfive\nsix\n";
        let mut editor = Editor::<()>::new(Rope::from_str(text));
        let n_chars = editor.buffer.len_chars() + 10;

        let mut positions = Vec::new();
        for _ in 0..n_chars {
            editor.action(Action::MoveCursorUp);
            positions.push(editor.cursor);
        }

        let zero_pos = positions.iter().position(|&p| p == 0).unwrap_or(usize::MAX);
        if zero_pos < positions.len() {
            for &p in &positions[zero_pos..] {
                assert_eq!(p, 0, "after hitting 0 the cursor should stay at 0");
            }
        }

        for w in positions.windows(2) {
            assert!(
                w[1] <= w[0],
                "cursor went backward from {} to {}",
                w[0],
                w[1],
            );
        }
    }

    /// Navigating up through an empty line that follows a long wrapped line
    /// should also be monotonic — no oscillation from VLine confusion.
    #[test]
    fn navigate_visual_through_empty_line_with_narrow_width() {
        let mut editor = Editor::<()>::new(Rope::from_str(""));
        // A long line that wraps, then an empty line, then another line.
        let text = "The quick brown fox jumps over the lazy dog\n\nEnd of document\n";
        for ch in text.chars() {
            editor.action(Action::Insert(ch));
        }

        // Set a narrow width to force the first line to wrap into many
        // visual lines.  The empty line and the final line follow.
        let narrow = 120.0;
        editor.nav_width.set(narrow);

        let mut positions = Vec::new();
        for _ in 0..text.chars().count() {
            editor.action(Action::MoveCursorUp);
            positions.push(editor.cursor);
        }

        for w in positions.windows(2) {
            assert!(
                w[1] <= w[0],
                "cursor went backward from {} to {}",
                w[0],
                w[1],
            );
        }
    }

    /// Moving the cursor with the keyboard past the bottom of the visible
    /// window should scroll the viewport to follow it, not leave it
    /// rendered off-screen.
    #[test]
    fn cursor_movement_scrolls_viewport_into_view_below() {
        let mut editor = Editor::<()>::new(Rope::from_str(""));
        let mut text = String::new();
        for i in 1..=50 {
            text.push_str(&format!("line {i}\n"));
        }
        for ch in text.chars() {
            editor.action(Action::Insert(ch));
        }
        editor.action(Action::MoveCursorLeft);

        {
            let element: iced::Element<'_, (), iced::Theme, iced::Renderer> = editor.view().into();
            let mut ui = iced_test::Simulator::with_size(
                iced_test::core::Settings::default(),
                iced_test::core::Size::new(200.0, 200.0),
                element,
            );
            let _ = ui.snapshot(&iced::Theme::Light);
        }
        // After the first layout pass, scroll-into-view moved the anchor
        // from 0 to follow the cursor at the end of the 50-line document.
        assert!(
            editor.scroll_anchor.get().0 > 0,
            "scroll-into-view should have moved the anchor past 0"
        );

        for _ in 0..(50 - 20) * 7 {
            editor.action(Action::MoveCursorUp);
        }
        for _ in 0..15 {
            editor.action(Action::MoveCursorDown);
        }
        let cursor_line = editor.buffer.byte_to_line(editor.flat_cursor());

        let element: iced::Element<'_, (), iced::Theme, iced::Renderer> = editor.view().into();
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced_test::core::Size::new(200.0, 200.0),
            element,
        );
        let _ = ui.snapshot(&iced::Theme::Light);

        let (anchor_line, _) = editor.scroll_anchor.get();
        assert!(
            cursor_line >= anchor_line,
            "cursor line {cursor_line} should be at or below the new anchor {anchor_line}"
        );
        assert!(
            anchor_line > 0,
            "viewport should have scrolled down to follow the cursor, stayed at {anchor_line}"
        );
    }

    /// Moving the cursor with the keyboard past the top of the visible
    /// window should scroll the viewport back up to follow it.
    #[test]
    fn cursor_movement_scrolls_viewport_into_view_above() {
        let mut editor = Editor::<()>::new(Rope::from_str(""));
        let mut text = String::new();
        for i in 1..=50 {
            text.push_str(&format!("line {i}\n"));
        }
        for ch in text.chars() {
            editor.action(Action::Insert(ch));
        }

        {
            let element: iced::Element<'_, (), iced::Theme, iced::Renderer> = editor.view().into();
            let mut ui = iced_test::Simulator::with_size(
                iced_test::core::Settings::default(),
                iced_test::core::Size::new(200.0, 200.0),
                element,
            );
            ui.point_at(iced_test::core::Point::new(50.0, 50.0));
            let _ = ui.snapshot(&iced::Theme::Light);
            ui.simulate([iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y: -5.0 },
            })]);
            let _ = ui.snapshot(&iced::Theme::Light);
        }
        let (anchor_before, _) = editor.scroll_anchor.get();
        assert!(
            anchor_before > 0,
            "should have scrolled down via the wheel first"
        );

        for _ in 0..text.chars().count() {
            editor.action(Action::MoveCursorLeft);
        }

        let element: iced::Element<'_, (), iced::Theme, iced::Renderer> = editor.view().into();
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced_test::core::Size::new(200.0, 200.0),
            element,
        );
        let _ = ui.snapshot(&iced::Theme::Light);

        assert_eq!(
            editor.scroll_anchor.get(),
            (0, 0.0),
            "viewport should have scrolled back up to reveal the cursor at the start"
        );
    }

    /// Regression test: typing that makes the cursor's own line word-wrap
    /// into extra visual rows — pushing its wrapped tail below the
    /// viewport's bottom edge — used to leave the viewport unmoved. The old
    /// scroll-into-view check only compared logical line *counts* against
    /// the shaped window; a single wrapped row growing past the window's
    /// pixel budget didn't change that count, so it looked "already in
    /// view" even once its bottom ran off-screen mid-keystroke.
    #[test]
    fn typing_wraps_line_past_viewport_bottom_scrolls_to_follow() {
        let mut editor = Editor::<()>::new(Rope::from_str(""));
        for ch in "short\n".chars() {
            editor.action(Action::Insert(ch));
        }
        // A long line typed at the end, narrow enough to wrap into many
        // more visual rows than a small viewport can show at once.
        for ch in "wrap ".repeat(20).chars() {
            editor.action(Action::Insert(ch));
        }

        let element: iced::Element<'_, (), iced::Theme, iced::Renderer> = editor.view().into();
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced_test::core::Size::new(150.0, 80.0),
            element,
        );
        let _ = ui.snapshot(&iced::Theme::Light);

        let (anchor_line, anchor_offset) = editor.scroll_anchor.get();
        assert!(
            anchor_line > 0 || anchor_offset > 0.0,
            "viewport should have scrolled to follow the cursor onto the wrapped \
             line's overflowing tail, anchor stayed at (0, 0.0)"
        );
    }

    /// Regression test: if the cursor's own line is only *partially* cut
    /// off by the viewport's top edge (e.g. left mid-scrolled by a small
    /// wheel nudge that didn't cross a whole line), the viewport should
    /// still realign so the line is fully visible rather than treating
    /// "the right logical line is somewhere in the window" as good enough.
    #[test]
    fn cursor_on_partially_scrolled_anchor_line_snaps_flush_to_top() {
        let mut editor = Editor::<()>::new(Rope::from_str(""));
        let mut text = String::new();
        for i in 1..=10 {
            text.push_str(&format!("line {i}\n"));
        }
        for ch in text.chars() {
            editor.action(Action::Insert(ch));
        }
        for _ in 0..text.chars().count() {
            editor.action(Action::MoveCursorLeft);
        }
        // Cursor is now at position 0, on line 0.

        {
            let element: iced::Element<'_, (), iced::Theme, iced::Renderer> = editor.view().into();
            let mut ui = iced_test::Simulator::with_size(
                iced_test::core::Settings::default(),
                iced_test::core::Size::new(200.0, 200.0),
                element,
            );
            ui.point_at(iced_test::core::Point::new(50.0, 50.0));
            let _ = ui.snapshot(&iced::Theme::Light);

            // A small downward nudge, well under one line's height, so the
            // anchor stays on line 0 but scrolls partway into it. The wheel
            // event's own immediate relayout is wheel-driven, so
            // scroll-into-view is deliberately suppressed for it — this
            // partially-scrolled state is expected to persist past it.
            ui.simulate([iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Pixels { x: 0.0, y: -10.0 },
            })]);
        }
        let (anchor_line, anchor_offset) = editor.scroll_anchor.get();
        assert_eq!(anchor_line, 0);
        assert!(
            anchor_offset > 0.0,
            "wheel nudge should have partially scrolled into line 0"
        );

        // A fresh layout pass (cursor still on line 0, not wheel-driven)
        // should snap the anchor flush to that line's top rather than
        // leaving it — and the cursor on it — cut off.
        let element: iced::Element<'_, (), iced::Theme, iced::Renderer> = editor.view().into();
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced_test::core::Size::new(200.0, 200.0),
            element,
        );
        let _ = ui.snapshot(&iced::Theme::Light);

        assert_eq!(
            editor.scroll_anchor.get(),
            (0, 0.0),
            "cursor's own line was partially cut off at the top; \
             the viewport should have snapped flush to it"
        );
    }
}
