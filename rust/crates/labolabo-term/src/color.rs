//! Backend-independent color configuration for a terminal session.
//!
//! This is the seam `labolabo-app` (or any future caller) uses to hand the
//! user's own terminal-emulator color preferences -- Ghostty's `background`/
//! `foreground`/`cursor-color`/`palette`/`theme` settings, in `labolabo-app`'s
//! case -- down to whichever [`crate::backend::VtBackend`] is active, without
//! either side needing to know about the other's config format or FFI types.

use crate::snapshot::Rgb;

/// User-configured color overrides for a terminal session. Every field is
/// optional/empty by default, meaning "leave the backend's own built-in
/// default alone" -- a session spawned with `ColorScheme::default()` (e.g.
/// via [`crate::TermSession::spawn`]/`spawn_with_command`) renders exactly as
/// it did before this type existed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorScheme {
    /// The default foreground color. `None` keeps the backend's built-in
    /// default (Ghostty's own default is white, `#FFFFFF`).
    pub foreground: Option<Rgb>,
    /// The default background color. `None` keeps the backend's built-in
    /// default.
    pub background: Option<Rgb>,
    /// The default cursor color. `None` keeps the backend's built-in
    /// default.
    pub cursor: Option<Rgb>,
    /// Overrides for the 256-color indexed palette, as `(index, color)`
    /// pairs. Entries are applied **in order**, so a later entry for the
    /// same index overrides an earlier one in this same `Vec` -- see
    /// [`Self::apply_palette`]. An index not present here keeps whatever the
    /// backend's own built-in palette has at that slot.
    pub palette: Vec<(u8, Rgb)>,
}

impl ColorScheme {
    /// Overlay [`Self::palette`]'s overrides onto a full 256-entry base
    /// table (a backend's own built-in default palette), applied in order so
    /// a later entry for a given index wins over an earlier one -- the same
    /// semantics as Ghostty's own `Palette.parseCLI`, which writes directly
    /// into a 256-element array (`self.value[entry.index] = entry.color`),
    /// so repeated config entries for the same index simply overwrite.
    pub fn apply_palette(&self, base: [Rgb; 256]) -> [Rgb; 256] {
        let mut resolved = base;
        for &(index, color) in &self.palette {
            resolved[index as usize] = color;
        }
        resolved
    }
}
