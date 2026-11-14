//! Part of `bidi.ts`: the text direction. The bidi algorithm comes with
//! the view's line rendering.

/// A text direction (`Direction`). The values are the bidi base levels of
/// spans in that direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Left-to-right.
    Ltr = 0,
    /// Right-to-left.
    Rtl = 1,
}
