//! Turning stored text into the representation a widget draws.
//!
//! A [`Row`] is that representation: an ordered list of [`Fragment`]s, each
//! carrying its text, its appearance, and the document bytes it stands for
//! — or nothing, when it stands for no document bytes at all.
//!
//! [`Layer`]s produce it. Each one receives the row built so far and
//! rewrites it, so they stack: put [`Plain`] at the bottom to read the
//! line out of storage, then whatever restyles, hides or adds to it.
//!
//! ```
//! use sputnik_editor::core::{Layer, Plain, Row, Style, Text};
//!
//! /// Hides the leading `#` of a markdown heading and enlarges the rest.
//! struct Heading;
//!
//! impl<T: Text + ?Sized> Layer<T> for Heading {
//!     fn apply<'a>(&self, text: &'a T, row: &mut Row<'a>) {
//!         let start = text.line_start(row.line);
//!         if text.substring(start..text.line_end(row.line)).starts_with("# ") {
//!             row.conceal(start..start + 2);
//!             row.style(start..text.line_end(row.line), |style| {
//!                 style.scale = Some(1.8);
//!             });
//!         }
//!     }
//! }
//!
//! let text = String::from("# Title");
//! let mut row = Row::new(0);
//! (Plain, Heading).apply(&text, &mut row);
//!
//! assert_eq!(row.text(), "Title");
//! assert_eq!(row.fragments[0].style.scale, Some(1.8));
//! ```

use std::borrow::Cow;
use std::hash::{Hash, Hasher};
use std::ops::Range;

use crate::core::{Style, Text};

/// One run of text within a [`Row`], sharing a single [`Style`].
///
/// Text is borrowed from storage wherever possible, so an unstyled line
/// costs no copying at all.
#[derive(Debug, Clone, PartialEq)]
pub struct Fragment<'a> {
    /// What gets drawn.
    pub text: Cow<'a, str>,
    /// The document bytes this run stands for, or `None` when it exists
    /// only on screen.
    ///
    /// When the range's length equals the text's, positions map through
    /// one to one and a caret can sit anywhere inside. When they differ
    /// the run is a substitution, and positions inside it resolve to
    /// whichever end is nearer.
    pub source: Option<Range<usize>>,
    /// How it should look.
    pub style: Style,
}

impl<'a> Fragment<'a> {
    /// A run that renders document bytes as they are stored.
    ///
    /// ```
    /// use sputnik_editor::core::{Fragment, Style};
    ///
    /// let fragment = Fragment::source(4..7, "abc", Style::default());
    /// assert_eq!(fragment.source, Some(4..7));
    /// ```
    pub fn source(source: Range<usize>, text: impl Into<Cow<'a, str>>, style: Style) -> Self {
        Self {
            text: text.into(),
            source: Some(source),
            style,
        }
    }

    /// A run that exists only on screen, standing for no document bytes.
    ///
    /// ```
    /// use sputnik_editor::core::{Fragment, Style};
    ///
    /// let fragment = Fragment::inserted("→", Style::default());
    /// assert_eq!(fragment.source, None);
    /// ```
    pub fn inserted(text: impl Into<Cow<'a, str>>, style: Style) -> Self {
        Self {
            text: text.into(),
            source: None,
            style,
        }
    }

    /// Whether positions map through this run one to one.
    pub fn is_verbatim(&self) -> bool {
        self.source
            .as_ref()
            .is_some_and(|source| source.len() == self.text.len())
    }
}

/// One line of a document, as it will be drawn.
///
/// Fragments appear in drawing order, and the source ranges of those that
/// have one never move backwards. Both properties are what let
/// [`Mapping`] translate between screen offsets and document positions.
///
/// # Examples
///
/// ```
/// use sputnik_editor::core::{Layer, Plain, Row, Style, Text};
///
/// let text = String::from("hello world");
/// let mut row = Row::new(0);
/// Plain.apply(&text, &mut row);
///
/// row.style(0..5, |style| style.bold = true);
/// row.insert(6, "» ", Style::default());
/// row.conceal(6..11);
///
/// assert_eq!(row.text(), "hello » ");
/// assert!(row.fragments[0].style.bold);
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row<'a> {
    /// The line of the document being rendered.
    pub line: usize,
    /// Its runs, in drawing order.
    pub fragments: Vec<Fragment<'a>>,
}

impl<'a> Row<'a> {
    /// An empty row for `line`, ready for a [`Layer`] stack to fill.
    pub fn new(line: usize) -> Self {
        Self {
            line,
            fragments: Vec::new(),
        }
    }

    /// Empties the row and points it at `line`, keeping the allocation.
    ///
    /// Reuse one row across every line of a frame and rendering allocates
    /// nothing per line.
    pub fn reset(&mut self, line: usize) {
        self.line = line;
        self.fragments.clear();
    }

    /// Whether nothing at all will be drawn.
    pub fn is_empty(&self) -> bool {
        self.fragments
            .iter()
            .all(|fragment| fragment.text.is_empty())
    }

    /// Total length in bytes of the text to be drawn.
    pub fn len(&self) -> usize {
        self.fragments
            .iter()
            .map(|fragment| fragment.text.len())
            .sum()
    }

    /// The text to be drawn, concatenated.
    ///
    /// Allocates; the widget shapes the fragments directly instead. Handy
    /// in tests and for layers that need to look at what came before them.
    pub fn text(&self) -> String {
        self.fragments
            .iter()
            .map(|fragment| fragment.text.as_ref())
            .collect()
    }

    /// Ensures a fragment boundary at document position `at`, and returns
    /// the index of the first fragment at or after it.
    ///
    /// Runs that render their source verbatim are cut in two; substitutions
    /// and inserted text are indivisible and are stepped over whole.
    ///
    /// ```
    /// use sputnik_editor::core::{Layer, Plain, Row, Text};
    ///
    /// let text = String::from("hello");
    /// let mut row = Row::new(0);
    /// Plain.apply(&text, &mut row);
    ///
    /// assert_eq!(row.split(2), 1);
    /// assert_eq!(row.fragments.len(), 2);
    /// assert_eq!(row.fragments[0].text, "he");
    /// assert_eq!(row.fragments[1].text, "llo");
    /// ```
    pub fn split(&mut self, at: usize) -> usize {
        for index in 0..self.fragments.len() {
            let Some(source) = self.fragments[index].source.clone() else {
                continue;
            };
            if at <= source.start {
                return index;
            }
            if at >= source.end {
                continue;
            }
            if !self.fragments[index].is_verbatim() {
                return index + 1;
            }

            let local = at - source.start;
            let fragment = &mut self.fragments[index];
            let tail = Fragment {
                text: split_off(&mut fragment.text, local),
                source: Some(at..source.end),
                style: fragment.style,
            };
            fragment.source = Some(source.start..at);
            self.fragments.insert(index + 1, tail);
            return index + 1;
        }
        self.fragments.len()
    }

    /// Restyles the runs covering the document bytes in `range`, splitting
    /// runs that only partly overlap it.
    ///
    /// ```
    /// use sputnik_editor::core::{Color, Layer, Plain, Row, Text};
    ///
    /// let text = String::from("let x = 1");
    /// let mut row = Row::new(0);
    /// Plain.apply(&text, &mut row);
    ///
    /// row.style(0..3, |style| style.color = Some(Color::rgb(0.8, 0.3, 0.2)));
    ///
    /// assert_eq!(row.fragments[0].text, "let");
    /// assert!(row.fragments[0].style.color.is_some());
    /// assert!(row.fragments[1].style.color.is_none());
    /// ```
    pub fn style(&mut self, range: Range<usize>, restyle: impl Fn(&mut Style)) {
        let start = self.split(range.start);
        let end = self.split(range.end);
        for fragment in &mut self.fragments[start..end] {
            restyle(&mut fragment.style);
        }
    }

    /// Drops the runs covering the document bytes in `range`, so they are
    /// not drawn at all.
    ///
    /// The bytes stay in the document, and stay addressable: a caret can
    /// still be moved into them, and [`Mapping`] keeps it visible by
    /// resolving it against the drawn text beside it. Several presses of
    /// an arrow key can therefore cross concealed text without the caret
    /// appearing to move.
    ///
    /// ```
    /// use sputnik_editor::core::{Layer, Plain, Row, Text};
    ///
    /// let text = String::from("**bold**");
    /// let mut row = Row::new(0);
    /// Plain.apply(&text, &mut row);
    ///
    /// row.conceal(6..8);
    /// row.conceal(0..2);
    ///
    /// assert_eq!(row.text(), "bold");
    /// ```
    pub fn conceal(&mut self, range: Range<usize>) {
        let start = self.split(range.start);
        let end = self.split(range.end);
        self.fragments.drain(start..end);
    }

    /// Adds text that is not in the document, to be drawn at document
    /// position `at`.
    ///
    /// ```
    /// use sputnik_editor::core::{Layer, Plain, Row, Style, Text};
    ///
    /// let text = String::from("TODO fix this");
    /// let mut row = Row::new(0);
    /// Plain.apply(&text, &mut row);
    ///
    /// row.insert(0, "⚠ ", Style::default());
    ///
    /// assert_eq!(row.text(), "⚠ TODO fix this");
    /// ```
    pub fn insert(&mut self, at: usize, text: impl Into<Cow<'a, str>>, style: Style) {
        let index = self.split(at);
        self.fragments
            .insert(index, Fragment::inserted(text, style));
    }

    /// Replaces the document bytes in `range` with different text, keeping
    /// them addressable.
    ///
    /// Unlike [`Row::conceal`] followed by [`Row::insert`], the new text
    /// stays tied to the bytes it stands for, so a click on it resolves
    /// into the range rather than beside it.
    ///
    /// ```
    /// use sputnik_editor::core::{Layer, Plain, Row, Style, Text};
    ///
    /// let text = String::from("a -> b");
    /// let mut row = Row::new(0);
    /// Plain.apply(&text, &mut row);
    ///
    /// row.replace(2..4, "→", Style::default());
    ///
    /// assert_eq!(row.text(), "a → b");
    /// ```
    pub fn replace(&mut self, range: Range<usize>, text: impl Into<Cow<'a, str>>, style: Style) {
        let start = self.split(range.start);
        let end = self.split(range.end);
        self.fragments.drain(start..end);
        self.fragments
            .insert(start, Fragment::source(range, text, style));
    }
}

/// Splits `text` at `at`, leaving the head in place and returning the tail.
fn split_off<'a>(text: &mut Cow<'a, str>, at: usize) -> Cow<'a, str> {
    match text {
        Cow::Borrowed(borrowed) => {
            let (head, tail) = borrowed.split_at(at);
            *text = Cow::Borrowed(head);
            Cow::Borrowed(tail)
        }
        Cow::Owned(owned) => {
            let tail = owned.split_off(at);
            Cow::Owned(tail)
        }
    }
}

/// A stackable transformation from stored text into a drawable [`Row`].
///
/// Each layer receives the row produced by the ones below it and rewrites
/// it in place, which is what makes them compose: order the stack from the
/// most general to the most specific and every layer sees the result of
/// its predecessors.
///
/// Implement it once, generically over storage, and it works with any
/// [`Text`].
///
/// # Examples
///
/// A layer that greys out trailing whitespace:
///
/// ```
/// use sputnik_editor::core::{Color, Layer, Plain, Row, Text};
///
/// struct TrailingSpace;
///
/// impl<T: Text + ?Sized> Layer<T> for TrailingSpace {
///     fn apply<'a>(&self, text: &'a T, row: &mut Row<'a>) {
///         let start = text.line_start(row.line);
///         let end = text.line_end(row.line);
///         let line = text.substring(start..end);
///         let trimmed = line.trim_end().len();
///         if trimmed < line.len() {
///             row.style(start + trimmed..end, |style| {
///                 style.color = Some(Color::rgba(0.5, 0.5, 0.5, 0.3));
///             });
///         }
///     }
/// }
///
/// let text = String::from("code   ");
/// let mut row = Row::new(0);
/// (Plain, TrailingSpace).apply(&text, &mut row);
///
/// assert_eq!(row.fragments.len(), 2);
/// assert!(row.fragments[1].style.color.is_some());
/// ```
pub trait Layer<T: Text + ?Sized> {
    /// Rewrites `row` for the line it names, reading `text` as needed.
    fn apply<'a>(&self, text: &'a T, row: &mut Row<'a>);
}

/// The bottom of every stack: reads a line out of storage verbatim.
///
/// Borrows directly from the text, so an unstyled document is rendered
/// without copying a single byte.
///
/// # Examples
///
/// ```
/// use sputnik_editor::core::{Layer, Plain, Row, Text};
///
/// let text = String::from("first\nsecond");
/// let mut row = Row::new(1);
/// Plain.apply(&text, &mut row);
///
/// assert_eq!(row.text(), "second");
/// assert_eq!(row.fragments[0].source, Some(6..12));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Plain;

impl<T: Text + ?Sized> Layer<T> for Plain {
    fn apply<'a>(&self, text: &'a T, row: &mut Row<'a>) {
        let mut at = text.line_start(row.line);
        for chunk in text.chunks(at..text.line_end(row.line)) {
            if chunk.is_empty() {
                continue;
            }
            row.fragments.push(Fragment::source(
                at..at + chunk.len(),
                chunk,
                Style::default(),
            ));
            at += chunk.len();
        }
    }
}

/// A stack of layers chosen at run time.
///
/// Tuples already stack layers known at compile time — `(Plain, Syntax)`
/// is a [`Layer`] — so reach for this when the set is configurable, and
/// nest freely since a stack is itself a layer.
///
/// A stack without [`Plain`] at the bottom draws nothing at all, since
/// nothing has read the text; an empty stack draws nothing for the same
/// reason.
///
/// # Examples
///
/// ```
/// use sputnik_editor::core::{Layer, Plain, Row, Stack, Style, Text};
///
/// struct Bullet;
///
/// impl<T: Text + ?Sized> Layer<T> for Bullet {
///     fn apply<'a>(&self, text: &'a T, row: &mut Row<'a>) {
///         row.insert(text.line_start(row.line), "• ", Style::default());
///     }
/// }
///
/// let stack = Stack::new().with(Plain).with(Bullet);
///
/// let text = String::from("item");
/// let mut row = Row::new(0);
/// stack.apply(&text, &mut row);
///
/// assert_eq!(row.text(), "• item");
/// ```
pub struct Stack<T: Text + ?Sized> {
    layers: Vec<Box<dyn Layer<T>>>,
}

impl<T: Text + ?Sized> Stack<T> {
    /// An empty stack, which renders nothing until a layer is added.
    pub fn new() -> Self {
        Self { layers: Vec::new() }
    }

    /// Adds a layer on top of the ones already present.
    pub fn with(mut self, layer: impl Layer<T> + 'static) -> Self {
        self.layers.push(Box::new(layer));
        self
    }

    /// Adds a layer on top of the ones already present.
    pub fn push(&mut self, layer: impl Layer<T> + 'static) {
        self.layers.push(Box::new(layer));
    }

    /// How many layers the stack holds.
    pub fn len(&self) -> usize {
        self.layers.len()
    }

    /// Whether the stack holds no layers.
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
}

impl<T: Text + ?Sized> Default for Stack<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Text + ?Sized> std::fmt::Debug for Stack<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Stack")
            .field("layers", &self.layers.len())
            .finish()
    }
}

impl<T: Text + ?Sized> Layer<T> for Stack<T> {
    fn apply<'a>(&self, text: &'a T, row: &mut Row<'a>) {
        for layer in &self.layers {
            layer.apply(text, row);
        }
    }
}

impl<T: Text + ?Sized, L: Layer<T> + ?Sized> Layer<T> for &L {
    fn apply<'a>(&self, text: &'a T, row: &mut Row<'a>) {
        (*self).apply(text, row);
    }
}

impl<T: Text + ?Sized, L: Layer<T> + ?Sized> Layer<T> for Box<L> {
    fn apply<'a>(&self, text: &'a T, row: &mut Row<'a>) {
        (**self).apply(text, row);
    }
}

/// The identity layer, leaving a row exactly as it found it.
impl<T: Text + ?Sized> Layer<T> for () {
    fn apply<'a>(&self, _text: &'a T, _row: &mut Row<'a>) {}
}

macro_rules! stack_tuple {
    ($($name:ident),+) => {
        impl<T: Text + ?Sized, $($name: Layer<T>),+> Layer<T> for ($($name,)+) {
            fn apply<'a>(&self, text: &'a T, row: &mut Row<'a>) {
                #[allow(non_snake_case)]
                let ($($name,)+) = self;
                $($name.apply(text, row);)+
            }
        }
    };
}

stack_tuple!(A);
stack_tuple!(A, B);
stack_tuple!(A, B, C);
stack_tuple!(A, B, C, D);
stack_tuple!(A, B, C, D, E);
stack_tuple!(A, B, C, D, E, F);

/// Translation between drawn offsets and document positions for one row.
///
/// Owned and self-contained, so a widget can keep it alongside a shaped
/// paragraph for as long as that paragraph lives.
///
/// Because layers may hide, add and substitute text, the two coordinate
/// spaces are not the same. Every position still resolves, though:
///
/// - Inside a run that renders its source verbatim, the two map one to one.
/// - Inside a substitution, a drawn offset resolves to whichever end of the
///   substituted range is nearer.
/// - Inside added text, a drawn offset resolves to where the document text
///   around it meets: the end of what is drawn before it, or the start of
///   what is drawn after it when nothing is.
/// - A document position that is hidden, or lies before or after everything
///   drawn, resolves to the nearest offset that is drawn, so a caret is
///   never lost and a selection never breaks apart.
///
/// # Examples
///
/// ```
/// use sputnik_editor::core::{Layer, Mapping, Plain, Row, Style, Text};
///
/// let text = String::from("hidden tail");
/// let mut row = Row::new(0);
/// Plain.apply(&text, &mut row);
/// row.conceal(6..11);
///
/// let mapping = Mapping::new(&row, 0);
///
/// assert_eq!(mapping.to_drawn(3), 3);
/// assert_eq!(mapping.to_source(3), 3);
/// // Everything hidden collapses onto the point where it was cut out.
/// assert_eq!(mapping.to_drawn(9), 6);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mapping {
    kind: Kind,
    len: usize,
    anchor: usize,
}

/// How much work translating a position actually takes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum Kind {
    /// Nothing that came from the document is drawn.
    #[default]
    Blank,
    /// Everything drawn is document text, in order and unbroken, so the two
    /// coordinate spaces differ by a constant.
    ///
    /// This is what an unstyled line collapses to — including one split
    /// across several storage chunks, since consecutive runs are merged —
    /// and it is why an ordinary document maps positions without a single
    /// allocation.
    Verbatim(Range<usize>),
    /// Anything a constant offset cannot express.
    Mapped(Vec<Entry>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    drawn: Range<usize>,
    source: Option<Range<usize>>,
}

impl Mapping {
    /// Builds the translation for `row`, whose document line starts at
    /// `anchor` — the position everything resolves to when a row draws
    /// nothing that came from the document.
    pub fn new(row: &Row<'_>, anchor: usize) -> Self {
        let mut len = 0;
        let mut direct: Option<Range<usize>> = None;
        let mut simple = true;

        for fragment in &row.fragments {
            len += fragment.text.len();
            if !simple {
                continue;
            }
            match &fragment.source {
                Some(source) if fragment.is_verbatim() => {
                    direct = match direct {
                        None => Some(source.clone()),
                        Some(seen) if seen.end == source.start => Some(seen.start..source.end),
                        Some(_) => {
                            simple = false;
                            None
                        }
                    };
                }
                _ => {
                    simple = false;
                    direct = None;
                }
            }
        }

        let kind = match (simple, direct) {
            (true, Some(source)) => Kind::Verbatim(source),
            (true, None) => Kind::Blank,
            (false, _) => Kind::Mapped(entries(row)),
        };

        Self { kind, len, anchor }
    }

    /// Total length in bytes of the drawn text.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether nothing is drawn.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether translation is a constant offset, which is the case that
    /// costs no allocation and no scanning.
    ///
    /// ```
    /// use sputnik_editor::core::{Layer, Mapping, Plain, Row, Style, Text};
    ///
    /// let text = String::from("plain line");
    /// let mut row = Row::new(0);
    /// Plain.apply(&text, &mut row);
    /// assert!(Mapping::new(&row, 0).is_direct());
    ///
    /// row.insert(4, "!", Style::default());
    /// assert!(!Mapping::new(&row, 0).is_direct());
    /// ```
    pub fn is_direct(&self) -> bool {
        matches!(self.kind, Kind::Verbatim(_) | Kind::Blank)
    }

    /// The document bytes this row covers, ignoring anything added to it.
    ///
    /// `None` when the row draws nothing that came from the document.
    pub fn source_range(&self) -> Option<Range<usize>> {
        match &self.kind {
            Kind::Blank => None,
            Kind::Verbatim(source) => Some(source.clone()),
            Kind::Mapped(entries) => {
                let mut sources = entries.iter().filter_map(|entry| entry.source.clone());
                let first = sources.next()?;
                Some(first.start..sources.next_back().map_or(first.end, |last| last.end))
            }
        }
    }

    /// The document position a drawn offset corresponds to.
    pub fn to_source(&self, offset: usize) -> usize {
        let entries = match &self.kind {
            Kind::Blank => return self.anchor,
            Kind::Verbatim(source) => return source.start + offset.min(self.len),
            Kind::Mapped(entries) => entries,
        };

        let mut preceding = None;

        for entry in entries {
            let contains = offset < entry.drawn.end;
            match &entry.source {
                Some(source) if contains => {
                    return if entry.drawn.len() == source.len() {
                        source.start + (offset - entry.drawn.start)
                    } else if offset - entry.drawn.start < entry.drawn.len().div_ceil(2) {
                        source.start
                    } else {
                        source.end
                    };
                }
                // Added text stands at the point it was added, which is
                // where the document text around it meets.
                None if contains => {
                    return preceding.unwrap_or_else(|| self.following(entry.drawn.end));
                }
                Some(source) => preceding = Some(source.end),
                None => {}
            }
        }

        preceding.unwrap_or(self.anchor)
    }

    /// The drawn offset a document position corresponds to.
    pub fn to_drawn(&self, at: usize) -> usize {
        let entries = match &self.kind {
            Kind::Blank => return 0,
            Kind::Verbatim(source) => return at.saturating_sub(source.start).min(self.len),
            Kind::Mapped(entries) => entries,
        };

        let mut preceding = None;

        for entry in entries {
            let Some(source) = &entry.source else {
                continue;
            };
            if at < source.start {
                // Hidden, or before everything: stay against whatever was
                // drawn last, so stepping the caret one character never
                // makes it jump over text that was added in between.
                return preceding.unwrap_or(entry.drawn.start);
            }
            if at < source.end {
                return if entry.drawn.len() == source.len() {
                    entry.drawn.start + (at - source.start)
                } else {
                    entry.drawn.start
                };
            }
            preceding = Some(entry.drawn.end);
        }

        preceding.unwrap_or(self.len)
    }

    /// The document position of the first run drawn at or after `offset`
    /// that came from the document.
    fn following(&self, offset: usize) -> usize {
        let Kind::Mapped(entries) = &self.kind else {
            return self.anchor;
        };
        entries
            .iter()
            .filter(|entry| entry.drawn.start >= offset)
            .find_map(|entry| entry.source.as_ref().map(|source| source.start))
            .unwrap_or(self.anchor)
    }
}

/// One entry per fragment, for rows a constant offset cannot express.
fn entries(row: &Row<'_>) -> Vec<Entry> {
    let mut entries = Vec::with_capacity(row.fragments.len());
    let mut offset = 0;

    for fragment in &row.fragments {
        let end = offset + fragment.text.len();
        entries.push(Entry {
            drawn: offset..end,
            source: fragment.source.clone(),
        });
        offset = end;
    }

    entries
}

/// A cheap summary of everything about a row that affects shaping.
///
/// Two rows with the same drawn text but different styling — or the same
/// styles cut at different points — produce different values, which is
/// what lets a shaped-paragraph cache be keyed on text without ever
/// returning one shaped under the wrong style.
///
/// # Examples
///
/// ```
/// use sputnik_editor::core::{Fingerprint, Layer, Plain, Row, Text};
///
/// let text = String::from("same");
/// let mut plain = Row::new(0);
/// Plain.apply(&text, &mut plain);
///
/// let mut bold = plain.clone();
/// bold.style(0..4, |style| style.bold = true);
///
/// assert_ne!(Fingerprint::of(&plain), Fingerprint::of(&bold));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Fingerprint(u64);

impl Fingerprint {
    /// Summarises `row`.
    pub fn of(row: &Row<'_>) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for fragment in &row.fragments {
            // The lengths are what distinguish the same styles applied at
            // different cut points.
            fragment.text.len().hash(&mut hasher);
            fragment.style.hash(&mut hasher);
        }
        Self(hasher.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stack that hides a middle range and adds text of its own, so the
    /// mapping has to cope with every kind of run at once.
    struct Decorate;

    impl<T: Text + ?Sized> Layer<T> for Decorate {
        fn apply<'a>(&self, _text: &'a T, row: &mut Row<'a>) {
            row.conceal(4..7);
            row.insert(7, "<>", Style::default());
        }
    }

    /// `abcdXYZefgh` with `XYZ` hidden and `<>` added after it, so one
    /// row exercises every kind of run at once.
    fn decorated(check: impl FnOnce(&str, &Row<'_>, &Mapping)) {
        let text = String::from("abcdXYZefgh");
        let mut row = Row::new(0);
        (Plain, Decorate).apply(&text, &mut row);
        let mapping = Mapping::new(&row, 0);
        check(&text, &row, &mapping);
    }

    #[test]
    fn layers_hide_and_add_within_one_row() {
        decorated(|_, row, _| assert_eq!(row.text(), "abcd<>efgh"));
    }

    /// The contract in one property: anything still drawn must survive a
    /// trip into screen coordinates and back unchanged.
    #[test]
    fn visible_positions_survive_a_round_trip() {
        decorated(|text, _, mapping| {
            let hidden = 4..7;
            for at in 0..=text.len() {
                if hidden.contains(&at) {
                    continue;
                }
                assert_eq!(
                    mapping.to_source(mapping.to_drawn(at)),
                    at,
                    "position {at} did not survive the round trip"
                );
            }
        });
    }

    #[test]
    fn hidden_positions_resolve_to_where_they_were_cut_out() {
        decorated(|_, _, mapping| {
            for at in 4..7 {
                assert_eq!(
                    mapping.to_drawn(at),
                    4,
                    "a caret inside hidden text must stay visible"
                );
            }
        });
    }

    #[test]
    fn added_text_resolves_to_where_the_document_text_around_it_meets() {
        // "<>" occupies drawn offsets 4..6, between text drawn for 0..4
        // and text drawn for 7..11.
        decorated(|_, _, mapping| {
            assert_eq!(mapping.to_source(4), 4);
            assert_eq!(mapping.to_source(5), 4);
        });
    }

    #[test]
    fn added_text_at_the_start_of_a_row_keeps_the_caret_after_it() {
        let text = String::from("item");
        let mut row = Row::new(0);
        Plain.apply(&text, &mut row);
        row.insert(0, "• ", Style::default());

        let mapping = Mapping::new(&row, 0);
        assert_eq!(row.text(), "• item");
        assert_eq!(
            mapping.to_drawn(0),
            "• ".len(),
            "the caret belongs after a bullet, not before it"
        );
        assert_eq!(mapping.to_source(0), 0);
    }

    #[test]
    fn a_selection_spanning_hidden_text_stays_contiguous() {
        decorated(|_, _, mapping| {
            let (start, end) = (mapping.to_drawn(2), mapping.to_drawn(9));
            assert!(
                start < end,
                "both ends must map through, or the highlight collapses"
            );
            assert_eq!((start, end), (2, 8));
        });
    }

    /// Substituted runs are indivisible, so concealing a range that only
    /// partly covers one must take it whole or leave it whole — never cut
    /// it — or the ascending source ranges that every mapping guarantee
    /// rests on would break.
    #[test]
    fn concealing_across_a_substitution_takes_it_whole_or_not_at_all() {
        let ascending = |row: &Row<'_>| {
            let sources: Vec<_> = row
                .fragments
                .iter()
                .filter_map(|fragment| fragment.source.clone())
                .collect();
            sources.windows(2).all(|pair| pair[0].end <= pair[1].start)
        };

        // Concealing from inside the substitution to past it.
        let text = String::from("a -> b");
        let mut row = Row::new(0);
        Plain.apply(&text, &mut row);
        row.replace(2..4, "→", Style::default());
        row.conceal(3..6);
        assert_eq!(row.text(), "a →", "the substitution survives whole");
        assert!(ascending(&row));
        assert_eq!(Mapping::new(&row, 0).source_range(), Some(0..4));

        // Concealing from before the substitution to inside it.
        let mut row = Row::new(0);
        Plain.apply(&text, &mut row);
        row.replace(2..4, "→", Style::default());
        row.conceal(1..3);
        assert_eq!(row.text(), "a b", "the substitution is taken whole");
        assert!(ascending(&row));
    }

    #[test]
    fn a_substitution_keeps_its_range_addressable() {
        let text = String::from("a -> b");
        let mut row = Row::new(0);
        (Plain,).apply(&text, &mut row);
        row.replace(2..4, "→", Style::default());

        let mapping = Mapping::new(&row, 0);
        assert_eq!(row.text(), "a → b");
        assert_eq!(mapping.to_drawn(2), 2);
        assert_eq!(mapping.to_drawn(3), 2);
        // Either end of the substituted range remains reachable.
        assert_eq!(mapping.to_source(2), 2);
        assert_eq!(mapping.to_source(4), 4);
        assert_eq!(mapping.source_range(), Some(0..6));
    }

    /// Concealed bytes remain addressable, so a caret crossing them does
    /// not appear to move. A stack concealing the start of a line wants a
    /// host that binds Home to a drawn position rather than to
    /// [`Motion::RowStart`](crate::core::Motion::RowStart).
    #[test]
    fn a_caret_crossing_concealed_text_shares_one_drawn_offset() {
        let text = String::from("**bold**");
        let mut row = Row::new(0);
        Plain.apply(&text, &mut row);
        row.conceal(6..8);
        row.conceal(0..2);

        let mapping = Mapping::new(&row, 0);
        assert_eq!(row.text(), "bold");
        for concealed in 0..=2 {
            assert_eq!(mapping.to_drawn(concealed), 0);
        }
        assert_eq!(mapping.to_drawn(3), 1, "the first drawn byte follows");
    }

    /// The allocation-free path has to cover the case that dominates:
    /// an ordinary line, including one storage split across chunks.
    #[test]
    fn an_unstyled_line_maps_by_a_constant_offset() {
        let text = String::from("first\nsecond line");
        let mut row = Row::new(1);
        Plain.apply(&text, &mut row);

        let mapping = Mapping::new(&row, 6);
        assert!(mapping.is_direct());
        assert_eq!(mapping.source_range(), Some(6..17));
        assert_eq!(mapping.to_drawn(9), 3);
        assert_eq!(mapping.to_source(3), 9);
        // Positions outside the row clamp to its ends rather than escaping.
        assert_eq!(mapping.to_drawn(0), 0);
        assert_eq!(mapping.to_drawn(999), mapping.len());
    }

    /// Consecutive runs are merged, so a row assembled from several
    /// fragments still maps by a constant offset.
    #[test]
    fn adjacent_verbatim_runs_collapse_into_one_offset() {
        let text = String::from("abcdef");
        let mut row = Row::new(0);
        Plain.apply(&text, &mut row);
        row.split(2);
        row.split(4);

        assert_eq!(row.fragments.len(), 3);
        let mapping = Mapping::new(&row, 0);
        assert!(
            mapping.is_direct(),
            "runs that are contiguous in both spaces describe one offset"
        );
        for at in 0..=6 {
            assert_eq!(mapping.to_source(mapping.to_drawn(at)), at);
        }
    }

    #[test]
    fn an_entirely_hidden_row_still_resolves() {
        let text = String::from("gone");
        let mut row = Row::new(0);
        Plain.apply(&text, &mut row);
        row.conceal(0..4);

        let mapping = Mapping::new(&row, 0);
        assert!(mapping.is_empty());
        assert_eq!(mapping.source_range(), None);
        assert_eq!(mapping.to_source(0), 0);
        assert_eq!(mapping.to_drawn(2), 0);
    }

    #[test]
    fn fingerprints_separate_identical_text_cut_at_different_points() {
        let text = String::from("abc");

        let mut early = Row::new(0);
        Plain.apply(&text, &mut early);
        early.style(0..1, |style| style.bold = true);

        let mut late = Row::new(0);
        Plain.apply(&text, &mut late);
        late.style(0..2, |style| style.bold = true);

        assert_eq!(early.text(), late.text());
        assert_ne!(Fingerprint::of(&early), Fingerprint::of(&late));
    }

    #[test]
    fn a_reset_row_keeps_its_allocation() {
        let text = String::from("one\ntwo");
        let mut row = Row::new(0);
        Plain.apply(&text, &mut row);
        let capacity = row.fragments.capacity();

        row.reset(1);
        Plain.apply(&text, &mut row);

        assert_eq!(row.text(), "two");
        assert_eq!(row.fragments.capacity(), capacity);
    }
}
