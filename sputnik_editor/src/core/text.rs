//! Pluggable text storage.

use std::ops::Range;

/// Storage that a [`Document`](crate::core::Document) can read and edit.
///
/// Every position is a **byte offset** into the whole text, always on a
/// `char` boundary. Bytes are the unit that shaping, hit-testing and
/// incremental parsers all speak, so using them here means conversion
/// happens once per edit rather than once per row of every frame.
///
/// Only seven methods are required; the rest are derived from them and
/// exist so that an implementation gets word motion, character stepping
/// and chunked reading for free. Override a derived method whenever the
/// backing store can answer it faster.
///
/// # Shadowing
///
/// Storage types tend to have inherent methods of the same names — and
/// occasionally of different meaning, as `Rope::insert` counts in
/// characters where this counts in bytes. An inherent method always wins
/// method-call syntax, so name the trait explicitly when it matters:
/// `Text::insert(&mut text, at, "…")`.
///
/// # Examples
///
/// ```
/// use sputnik_editor::core::Text;
///
/// let mut text = String::from("hello");
/// Text::insert(&mut text, 5, ", world");
///
/// assert_eq!(text, "hello, world");
/// assert_eq!(Text::len(&text), 12);
/// assert_eq!(text.next_word(0), 5);
/// assert_eq!(text.substring(7..12), "world");
/// ```
pub trait Text {
    /// Builds storage holding `text`.
    ///
    /// ```
    /// use sputnik_editor::core::Text;
    ///
    /// let text = <String as Text>::from_str("hi");
    /// assert_eq!(Text::len(&text), 2);
    /// ```
    fn from_str(text: &str) -> Self
    where
        Self: Sized;

    /// Total length in bytes.
    fn len(&self) -> usize;

    /// Number of lines, counting the empty one after a trailing newline.
    ///
    /// Never zero: empty storage is one empty line.
    fn line_count(&self) -> usize;

    /// The line `at` falls on. Out-of-range positions clamp.
    fn line_of(&self, at: usize) -> usize;

    /// First byte of `line`. Out-of-range lines clamp to the last one.
    fn line_start(&self, line: usize) -> usize;

    /// The contiguous run of text containing `at`, and the byte offset at
    /// which that run begins.
    ///
    /// This is the single primitive every read is built from. Storage that
    /// keeps text in one piece returns the whole thing with an offset of
    /// zero; a rope returns the leaf containing `at`.
    ///
    /// The returned offset is always at or before `at`. When `at` is the
    /// very end of the text, the final run is returned, so `at` may equal
    /// `offset + run.len()`.
    fn chunk_at(&self, at: usize) -> (&str, usize);

    /// Inserts `text` at `at`.
    fn insert(&mut self, at: usize, text: &str);

    /// Removes `range`. An empty or reversed range is a no-op.
    fn remove(&mut self, range: Range<usize>);

    /// Whether there is no text at all.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The contiguous runs covering `range`, in order.
    ///
    /// Zero-copy: each run borrows directly from the storage. Iterating
    /// backwards is supported and reads only the runs it needs.
    ///
    /// ```
    /// use sputnik_editor::core::Text;
    ///
    /// let text = String::from("abcdef");
    /// let read: String = text.chunks(1..4).collect();
    /// assert_eq!(read, "bcd");
    /// ```
    fn chunks(&self, range: Range<usize>) -> Chunks<'_, Self> {
        let start = self.clamp(range.start);
        Chunks {
            text: self,
            start,
            end: self.clamp(range.end).max(start),
        }
    }

    /// `range` as an owned string.
    ///
    /// ```
    /// use sputnik_editor::core::Text;
    ///
    /// let text = String::from("hello, world");
    /// assert_eq!(text.substring(7..12), "world");
    /// ```
    fn substring(&self, range: Range<usize>) -> String {
        self.chunks(range).collect()
    }

    /// Snaps `at` into `0..=len` and back onto the nearest character
    /// boundary at or before it, so a position of unknown provenance can
    /// never slice through a multi-byte character.
    ///
    /// ```
    /// use sputnik_editor::core::Text;
    ///
    /// let text = String::from("é!"); // "é" occupies bytes 0..2
    /// assert_eq!(Text::clamp(&text, 1), 0);
    /// assert_eq!(Text::clamp(&text, 99), 3);
    /// ```
    fn clamp(&self, at: usize) -> usize {
        let mut at = at.min(self.len());
        loop {
            let (chunk, offset) = self.chunk_at(at);
            let local = at - offset;
            if local >= chunk.len() || chunk.is_char_boundary(local) {
                return at;
            }
            at -= 1;
        }
    }

    /// The character boundary before `at`, or `at` itself at the start.
    fn prev(&self, at: usize) -> usize {
        let at = self.clamp(at);
        match self
            .chunks(0..at)
            .next_back()
            .and_then(|chunk| chunk.chars().next_back())
        {
            Some(character) => at - character.len_utf8(),
            None => at,
        }
    }

    /// The character boundary after `at`, or `at` itself at the end.
    fn next(&self, at: usize) -> usize {
        let at = self.clamp(at);
        match self
            .chunks(at..self.len())
            .next()
            .and_then(|chunk| chunk.chars().next())
        {
            Some(character) => at + character.len_utf8(),
            None => at,
        }
    }

    /// Byte just past the last visible character of `line`, before its line
    /// break, so a caret placed here sits at the end of the text rather
    /// than at the start of the next line.
    ///
    /// A `\r\n` pair counts as one break.
    ///
    /// ```
    /// use sputnik_editor::core::Text;
    ///
    /// let text = String::from("one\ntwo");
    /// assert_eq!(text.line_start(0), 0);
    /// assert_eq!(text.line_end(0), 3);
    /// ```
    fn line_end(&self, line: usize) -> usize {
        let start = self.line_start(line);
        let mut end = if line + 1 < self.line_count() {
            self.line_start(line + 1)
        } else {
            self.len()
        };

        let last = |end: usize| {
            self.chunks(start..end)
                .next_back()
                .and_then(|chunk| chunk.as_bytes().last().copied())
        };
        if end > start && last(end) == Some(b'\n') {
            end -= 1;
        }
        if end > start && last(end) == Some(b'\r') {
            end -= 1;
        }
        end
    }

    /// Start of the word before `at`: any whitespace directly behind the
    /// caret, then the whole run of same-class characters.
    ///
    /// Letters, digits and `_` form one class and punctuation another, so a
    /// caret crossing `foo(bar` stops at the parenthesis instead of
    /// skipping the whole token.
    ///
    /// ```
    /// use sputnik_editor::core::Text;
    ///
    /// let text = String::from("foo(bar baz)");
    /// assert_eq!(text.prev_word(11), 8);
    /// assert_eq!(text.prev_word(8), 4);
    /// ```
    fn prev_word(&self, at: usize) -> usize {
        let mut at = self.clamp(at);
        let mut characters = self
            .chunks(0..at)
            .rev()
            .flat_map(|chunk| chunk.chars().rev());

        let mut class = None;
        for character in characters.by_ref() {
            at -= character.len_utf8();
            if !character.is_whitespace() {
                class = Some(Class::of(character));
                break;
            }
        }
        let Some(class) = class else {
            return at;
        };

        for character in characters {
            if Class::of(character) != class {
                break;
            }
            at -= character.len_utf8();
        }
        at
    }

    /// End of the word after `at`, mirroring [`Text::prev_word`].
    ///
    /// ```
    /// use sputnik_editor::core::Text;
    ///
    /// let text = String::from("foo(bar baz)");
    /// assert_eq!(text.next_word(0), 3);
    /// assert_eq!(text.next_word(3), 4);
    /// ```
    fn next_word(&self, at: usize) -> usize {
        let mut at = self.clamp(at);
        let mut characters = self.chunks(at..self.len()).flat_map(str::chars);

        let mut class = None;
        for character in characters.by_ref() {
            at += character.len_utf8();
            if !character.is_whitespace() {
                class = Some(Class::of(character));
                break;
            }
        }
        let Some(class) = class else {
            return at;
        };

        for character in characters {
            if Class::of(character) != class {
                break;
            }
            at += character.len_utf8();
        }
        at
    }
}

/// The contiguous runs of a [`Text`] range, produced by [`Text::chunks`].
///
/// Yields borrowed slices in order, and can be walked from either end.
#[derive(Debug)]
pub struct Chunks<'a, T: ?Sized> {
    text: &'a T,
    start: usize,
    end: usize,
}

impl<'a, T: Text + ?Sized> Iterator for Chunks<'a, T> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        if self.start >= self.end {
            return None;
        }
        let (chunk, offset) = self.text.chunk_at(self.start);
        let from = self.start - offset;
        let to = (self.end - offset).min(chunk.len());
        self.start = offset + to;
        Some(&chunk[from..to])
    }
}

impl<'a, T: Text + ?Sized> DoubleEndedIterator for Chunks<'a, T> {
    fn next_back(&mut self) -> Option<&'a str> {
        if self.start >= self.end {
            return None;
        }
        let (chunk, offset) = self.text.chunk_at(self.end - 1);
        let from = self.start.max(offset) - offset;
        let to = self.end - offset;
        self.end = offset + from;
        Some(&chunk[from..to])
    }
}

/// Character classes for word motion. Runs of one class move as a unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Punctuation,
    Whitespace,
}

impl Class {
    fn of(character: char) -> Self {
        if character.is_whitespace() {
            Class::Whitespace
        } else if character.is_alphanumeric() || character == '_' {
            Class::Word
        } else {
            Class::Punctuation
        }
    }
}

impl Text for ropey::Rope {
    fn from_str(text: &str) -> Self {
        ropey::Rope::from_str(text)
    }

    fn len(&self) -> usize {
        self.len_bytes()
    }

    fn line_count(&self) -> usize {
        self.len_lines()
    }

    fn line_of(&self, at: usize) -> usize {
        self.byte_to_line(Text::clamp(self, at))
    }

    fn line_start(&self, line: usize) -> usize {
        self.line_to_byte(line.min(self.len_lines().saturating_sub(1)))
    }

    fn chunk_at(&self, at: usize) -> (&str, usize) {
        let (chunk, offset, _, _) = self.chunk_at_byte(at.min(self.len_bytes()));
        (chunk, offset)
    }

    fn insert(&mut self, at: usize, text: &str) {
        let at = Text::clamp(self, at);
        let at = self.byte_to_char(at);
        ropey::Rope::insert(self, at, text);
    }

    fn remove(&mut self, range: Range<usize>) {
        let start = Text::clamp(self, range.start);
        let end = Text::clamp(self, range.end).max(start);
        if start == end {
            return;
        }
        let range = self.byte_to_char(start)..self.byte_to_char(end);
        ropey::Rope::remove(self, range);
    }

    /// Answered from the rope's own byte/character index rather than by
    /// scanning backwards for a boundary.
    fn clamp(&self, at: usize) -> usize {
        let at = at.min(self.len_bytes());
        self.char_to_byte(self.byte_to_char(at))
    }
}

/// A single contiguous buffer.
///
/// Every read is one borrow with no seeking, which makes it the fastest
/// option for short texts and the obvious choice in tests. Line lookups
/// scan, so they cost O(len) each; prefer a rope once documents grow past
/// a few thousand lines.
impl Text for String {
    fn from_str(text: &str) -> Self {
        text.to_owned()
    }

    fn len(&self) -> usize {
        self.as_str().len()
    }

    fn line_count(&self) -> usize {
        self.as_bytes()
            .iter()
            .filter(|byte| **byte == b'\n')
            .count()
            + 1
    }

    fn line_of(&self, at: usize) -> usize {
        let at = Text::clamp(self, at);
        self.as_bytes()[..at]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count()
    }

    fn line_start(&self, line: usize) -> usize {
        if line == 0 {
            return 0;
        }
        let mut seen = 0;
        for (index, byte) in self.as_bytes().iter().enumerate() {
            if *byte == b'\n' {
                seen += 1;
                if seen == line {
                    return index + 1;
                }
            }
        }
        self.as_str().len()
    }

    fn chunk_at(&self, _at: usize) -> (&str, usize) {
        (self.as_str(), 0)
    }

    fn insert(&mut self, at: usize, text: &str) {
        let at = Text::clamp(self, at);
        self.insert_str(at, text);
    }

    fn remove(&mut self, range: Range<usize>) {
        let start = Text::clamp(self, range.start);
        let end = Text::clamp(self, range.end).max(start);
        self.replace_range(start..end, "");
    }

    /// Answered with `str::is_char_boundary` instead of chunk lookups.
    fn clamp(&self, at: usize) -> usize {
        let mut at = at.min(self.as_str().len());
        while !self.is_char_boundary(at) {
            at -= 1;
        }
        at
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;

    /// Each expectation runs against every implementation, so a derived
    /// method cannot quietly depend on one storage's chunking.
    macro_rules! for_each_storage {
        ($name:ident, $body:item) => {
            #[test]
            fn $name() {
                $body
                check::<Rope>();
                check::<String>();
            }
        };
    }

    for_each_storage! {
        positions_are_bytes_snapped_to_character_boundaries,
        fn check<T: Text>() {
            let text = T::from_str("é!"); // "é" occupies bytes 0..2
            assert_eq!(text.len(), 3);
            assert_eq!(text.clamp(1), 0);
            assert_eq!(text.clamp(2), 2);
            assert_eq!(text.clamp(999), 3);
            assert_eq!(text.next(0), 2);
            assert_eq!(text.prev(2), 0);
            assert_eq!(text.prev(0), 0);
            assert_eq!(text.next(3), 3);
        }
    }

    for_each_storage! {
        lines_exclude_their_break,
        fn check<T: Text>() {
            let text = T::from_str("one\ntwo\r\nthree");
            assert_eq!(text.line_count(), 3);
            for (line, expected) in [(0, "one"), (1, "two"), (2, "three")] {
                let range = text.line_start(line)..text.line_end(line);
                assert_eq!(text.substring(range), expected);
            }
            assert_eq!(text.line_of(5), 1);
        }
    }

    for_each_storage! {
        word_motion_stops_where_the_character_class_changes,
        fn check<T: Text>() {
            let text = T::from_str("foo(bar baz)");
            assert_eq!(text.next_word(0), 3);
            assert_eq!(text.next_word(3), 4);
            assert_eq!(text.next_word(4), 7);
            assert_eq!(text.next_word(7), 11);
            assert_eq!(text.prev_word(12), 11);
            assert_eq!(text.prev_word(11), 8);
            assert_eq!(text.prev_word(8), 4);
            assert_eq!(text.prev_word(0), 0);
        }
    }

    for_each_storage! {
        edits_accept_positions_of_unknown_provenance,
        fn check<T: Text>() {
            let mut text = T::from_str("hello");
            text.remove(3..999);
            assert_eq!(text.substring(0..text.len()), "hel");
            text.insert(999, "!");
            assert_eq!(text.substring(0..text.len()), "hel!");
            text.remove(2..2);
            assert_eq!(text.substring(0..text.len()), "hel!");
        }
    }

    /// A rope splits this across several leaves, so the range being
    /// reassembled correctly proves the chunk walk handles interior
    /// boundaries rather than getting one big slice.
    #[test]
    fn chunks_reassemble_a_range_from_either_end() {
        let source: String = (0..2000).map(|index| format!("line {index}\n")).collect();
        let rope = Rope::from_str(&source);
        let range = 137..9_001;

        assert!(
            Text::chunks(&rope, range.clone()).count() > 1,
            "the fixture must be large enough to span several chunks"
        );

        let forward: String = Text::chunks(&rope, range.clone()).collect();
        assert_eq!(forward, source[range.clone()]);

        let mut backward: Vec<&str> = Text::chunks(&rope, range.clone()).rev().collect();
        backward.reverse();
        assert_eq!(backward.concat(), source[range]);
    }
}
