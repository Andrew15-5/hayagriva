//! Mapping of values to the locations they originate from in source code.

use std::fmt::{self, Debug, Display, Formatter};
use std::ops::Range;

#[cfg(test)]
use std::cell::Cell;

#[cfg(test)]
thread_local! {
    static CMP_SPANS: Cell<bool> = Cell::new(true);
}

/// Annotate a value with a span.
pub trait SpanWith: Sized {
    /// Wraps `self` in a `Spanned` with the given span.
    fn span_with(self, span: impl Into<Span>) -> Spanned<Self> {
        Spanned::new(self, span)
    }
}

impl<T> SpanWith for T {}

/// Span offsetting.
pub trait Offset {
    /// Offset all spans contained in `Self` by the given position.
    fn offset(self, by: Pos) -> Self;
}

/// A vector of spanned values of type `T`.
pub type SpanVec<T> = Vec<Spanned<T>>;

impl<T> Offset for SpanVec<T> {
    fn offset(mut self, by: Pos) -> Self {
        for spanned in &mut self {
            spanned.span = spanned.span.offset(by);
        }
        self
    }
}

/// A value with the span it corresponds to in the source code.
#[derive(Default, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Spanned<T> {
    /// The spanned value.
    pub v: T,
    /// The location in source code of the value.
    pub span: Span,
}

impl<T> Spanned<T> {
    /// Create a new instance from a value and its span.
    pub fn new(v: T, span: impl Into<Span>) -> Self {
        Self { v, span: span.into() }
    }

    /// Create a new instance from a value with the zero span.
    pub fn zero(v: T) -> Self {
        Self { v, span: Span::ZERO }
    }

    /// Access the value.
    pub fn value(self) -> T {
        self.v
    }

    /// Convert from `&Spanned<T>` to `Spanned<&T>`
    pub fn as_ref(&self) -> Spanned<&T> {
        Spanned { v: &self.v, span: self.span }
    }

    /// Map the value using a function while keeping the span.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned { v: f(self.v), span: self.span }
    }

    /// Maps the span while keeping the value.
    pub fn map_span(mut self, f: impl FnOnce(Span) -> Span) -> Self {
        self.span = f(self.span);
        self
    }
}

impl<T> Spanned<Option<T>> {
    /// Swap the spanned and the option.
    pub fn transpose(self) -> Option<Spanned<T>> {
        let Spanned { v, span } = self;
        v.map(|v| v.span_with(span))
    }
}

impl<T> Offset for Spanned<T> {
    fn offset(self, by: Pos) -> Self {
        self.map_span(|span| span.offset(by))
    }
}

impl<T: Debug> Debug for Spanned<T> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        self.v.fmt(f)?;
        if f.alternate() {
            f.write_str(" ")?;
            self.span.fmt(f)?;
        }
        Ok(())
    }
}

/// Locates a slice of source code.
#[derive(Copy, Clone, Ord, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Span {
    /// The inclusive start position.
    pub start: Pos,
    /// The inclusive end position.
    pub end: Pos,
}

impl Span {
    /// The zero span.
    pub const ZERO: Self = Self { start: Pos::ZERO, end: Pos::ZERO };

    /// Create a new span from start and end positions.
    pub fn new(start: impl Into<Pos>, end: impl Into<Pos>) -> Self {
        Self { start: start.into(), end: end.into() }
    }

    /// Create a span including just a single position.
    pub fn at(pos: impl Into<Pos> + Copy) -> Self {
        Self::new(pos, pos)
    }

    /// Create a new span with the earlier start and later end position.
    pub fn join(self, other: Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// Expand a span by merging it with another span.
    pub fn expand(&mut self, other: Self) {
        *self = self.join(other)
    }

    /// Convert to a `Range<usize>` for indexing.
    pub fn to_range(self) -> Range<usize> {
        self.start.to_usize() .. self.end.to_usize()
    }

    /// When set to `false` comparisons with `PartialEq` ignore spans.
    #[cfg(test)]
    pub(crate) fn set_cmp(cmp: bool) {
        CMP_SPANS.with(|cell| cell.set(cmp));
    }
}

impl Offset for Span {
    fn offset(self, by: Pos) -> Self {
        Self {
            start: self.start.offset(by),
            end: self.end.offset(by),
        }
    }
}

impl Eq for Span {}

impl PartialEq for Span {
    fn eq(&self, other: &Self) -> bool {
        #[cfg(test)]
        if !CMP_SPANS.with(Cell::get) {
            return true;
        }

        self.start == other.start && self.end == other.end
    }
}

impl Default for Span {
    fn default() -> Self {
        Span::ZERO
    }
}

impl<T> From<T> for Span
where
    T: Into<Pos> + Copy,
{
    fn from(pos: T) -> Self {
        Self::at(pos)
    }
}

impl<T> From<Range<T>> for Span
where
    T: Into<Pos>,
{
    fn from(range: Range<T>) -> Self {
        Self::new(range.start, range.end)
    }
}

impl Debug for Span {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "<{:?}-{:?}>", self.start, self.end)
    }
}

/// A byte position in source code.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Pos(pub u32);

impl Pos {
    /// The zero position.
    pub const ZERO: Self = Self(0);

    /// Convert to a usize for indexing.
    pub fn to_usize(self) -> usize {
        self.0 as usize
    }
}

impl Offset for Pos {
    fn offset(self, by: Self) -> Self {
        Pos(self.0 + by.0)
    }
}

impl From<u32> for Pos {
    fn from(index: u32) -> Self {
        Self(index)
    }
}

impl From<i32> for Pos {
    fn from(index: i32) -> Self {
        Self(index as u32)
    }
}

impl From<usize> for Pos {
    fn from(index: usize) -> Self {
        Self(index as u32)
    }
}

impl Debug for Pos {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        Debug::fmt(&self.0, f)
    }
}

/// A one-indexed line-column position in source code.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Location {
    /// The one-indexed line.
    pub line: u32,
    /// The one-indexed column.
    pub column: u32,
}

impl Location {
    /// Create a new location from line and column.
    pub fn new(line: u32, column: u32) -> Self {
        Self { line, column }
    }
}

impl Debug for Location {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Display for Location {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}
