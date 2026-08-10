//! How a run of text should look.

use std::hash::{Hash, Hasher};

/// A straight RGBA colour, so styling never has to name a renderer's own
/// colour type.
///
/// # Examples
///
/// ```
/// use sputnik_editor::core::Color;
///
/// let comment = Color::rgb(0.4, 0.5, 0.4);
/// assert_eq!(comment.a, 1.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    /// Red, `0.0..=1.0`.
    pub r: f32,
    /// Green, `0.0..=1.0`.
    pub g: f32,
    /// Blue, `0.0..=1.0`.
    pub b: f32,
    /// Alpha, `0.0` transparent to `1.0` opaque.
    pub a: f32,
}

impl Color {
    /// An opaque colour.
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// A colour with explicit alpha.
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
}

impl Hash for Color {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        for channel in [self.r, self.g, self.b, self.a] {
            channel.to_bits().hash(hasher);
        }
    }
}

/// The appearance of one run of text.
///
/// Every field defaults to "inherit whatever the editor is using", so a
/// layer only states what it actually changes.
///
/// # Examples
///
/// ```
/// use sputnik_editor::core::{Color, Style};
///
/// let heading = Style {
///     scale: Some(1.8),
///     bold: true,
///     ..Style::default()
/// };
/// let keyword = Style::colored(Color::rgb(0.8, 0.3, 0.2));
///
/// assert_eq!(Style::default().scale, None);
/// assert!(heading.bold);
/// assert_eq!(keyword.scale, None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Style {
    /// Text colour, or `None` to inherit the editor's.
    pub color: Option<Color>,
    /// Multiplier on the editor's font size, or `None` to inherit it. A
    /// markdown heading is the same text at `Some(1.8)`.
    pub scale: Option<f32>,
    /// Whether to render in a bold weight.
    pub bold: bool,
    /// Whether to render in an italic slant.
    pub italic: bool,
}

impl Style {
    /// A style that only changes the colour.
    ///
    /// ```
    /// use sputnik_editor::core::{Color, Style};
    ///
    /// let style = Style::colored(Color::rgb(1.0, 0.0, 0.0));
    /// assert_eq!(style.color, Some(Color::rgb(1.0, 0.0, 0.0)));
    /// assert!(!style.bold);
    /// ```
    pub const fn colored(color: Color) -> Self {
        Self {
            color: Some(color),
            scale: None,
            bold: false,
            italic: false,
        }
    }
}

impl Hash for Style {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        self.color.hash(hasher);
        self.scale.map(f32::to_bits).hash(hasher);
        self.bold.hash(hasher);
        self.italic.hash(hasher);
    }
}
