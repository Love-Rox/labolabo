//! The running program's currently requested mouse-reporting configuration
//! (DECSET `9`/`1000`/`1002`/`1003` mouse-tracking modes, plus DECSET `1006`
//! SGR extended coordinates) -- queried the same way as
//! [`crate::TermSession::bracketed_paste`]/[`crate::TermSession::
//! alt_screen_active`] (a plain flag refreshed by the worker thread after
//! every processed PTY byte batch, read non-blockingly by the caller
//! thread), so callers (`labolabo-app`'s mouse-event routing) can decide
//! per-event whether a click/drag/scroll should be forwarded to the child
//! program (vim, tmux, Claude Code's own TUI, ...) instead of driving this
//! crate's own text-selection/scrollback UI.
//!
//! Names mirror `libghostty-vt`'s own `mouse::TrackingMode` (the intended
//! production backend, see `crate::backend::ghostty`) rather than inventing
//! new terminology.

/// Mouse tracking protocol currently requested by the running program.
///
/// The four DECSET modes are mutually exclusive in both backends (confirmed
/// by reading `alacritty_terminal`'s `set_private_mode` -- "Mouse protocols
/// are mutually exclusive", it clears `TermMode::MOUSE_MODE` before setting
/// the new bit -- and `libghostty-vt`'s underlying `flags.mouse_event` is a
/// single tagged field, not a bitmask), so at most one non-`Off` variant is
/// ever active at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MouseTracking {
    /// No mouse tracking mode active (DECSET `9`/`1000`/`1002`/`1003` all
    /// unset) -- the common case. Mouse events are handled entirely by this
    /// crate's caller (text selection, scrollback), never forwarded to the
    /// child.
    #[default]
    Off,
    /// DECSET `9` -- X10 compatibility mode: left/middle/right button
    /// presses only (no release, no motion, no modifiers).
    ///
    /// **Not tracked by the alacritty backend**: `alacritty_terminal`'s
    /// vendored `vte` ANSI parser (confirmed by reading its source,
    /// `vte::ansi::PrivateMode::from(u16)`) has no `NamedPrivateMode`
    /// variant for mode `9` at all, so setting it is silently ignored --
    /// [`crate::backend::alacritty::AlacrittyBackend::mouse_mode`] can never
    /// report this variant. The ghostty backend (the intended production
    /// one) does track it via `Mode::X10_MOUSE`.
    X10,
    /// DECSET `1000` -- normal tracking: press and release, no motion.
    Normal,
    /// DECSET `1002` -- button-event tracking: adds motion reports while a
    /// button is held (drag).
    Button,
    /// DECSET `1003` -- any-event tracking: adds motion reports even with no
    /// button held (hover).
    Any,
}

impl MouseTracking {
    /// Whether this tracking mode is active at all (anything but [`Off`](Self::Off)).
    pub fn is_active(self) -> bool {
        !matches!(self, MouseTracking::Off)
    }

    /// Whether this tracking mode reports motion at all -- while a button is
    /// held ([`Button`](Self::Button)) or even while none is
    /// ([`Any`](Self::Any)).
    pub fn reports_motion(self) -> bool {
        matches!(self, MouseTracking::Button | MouseTracking::Any)
    }

    /// Whether this tracking mode reports motion with *no* button held (a
    /// plain hover) -- [`Any`](Self::Any) only.
    pub fn reports_hover_motion(self) -> bool {
        matches!(self, MouseTracking::Any)
    }

    /// Packs into 3 bits (`0..=4`) for [`MouseMode::to_bits`]/[`MouseMode::
    /// from_bits`]'s single-byte representation.
    fn to_bits(self) -> u8 {
        match self {
            MouseTracking::Off => 0,
            MouseTracking::X10 => 1,
            MouseTracking::Normal => 2,
            MouseTracking::Button => 3,
            MouseTracking::Any => 4,
        }
    }

    /// Inverse of [`Self::to_bits`]. Any out-of-range value (never produced
    /// by this crate itself, but defensive against a stray bit) falls back
    /// to [`Off`](Self::Off) -- the safe "don't forward mouse events"
    /// default.
    fn from_bits(bits: u8) -> Self {
        match bits {
            1 => MouseTracking::X10,
            2 => MouseTracking::Normal,
            3 => MouseTracking::Button,
            4 => MouseTracking::Any,
            _ => MouseTracking::Off,
        }
    }
}

/// The running program's current mouse-reporting configuration: which
/// [`MouseTracking`] protocol (if any) is requested, and whether SGR
/// extended-coordinate encoding (DECSET `1006`) is also requested.
///
/// Queried via [`crate::TermSession::mouse_mode`] -- see that method's doc
/// comment for the refresh cadence. Packed into a single `u8` internally
/// (see [`Self::to_bits`]/[`Self::from_bits`]) so `TermSession` can publish
/// it through one `AtomicU8`, the same "cheap plain-data flag the worker
/// thread refreshes and the caller thread reads non-blockingly" shape
/// `bracketed_paste`/`alt_screen_active` already use with `AtomicBool`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MouseMode {
    pub tracking: MouseTracking,
    /// Whether DECSET `1006` (SGR extended mouse coordinates) is requested.
    /// Meaningless when `tracking` is [`MouseTracking::Off`] -- a caller
    /// only needs to check this once `tracking.is_active()`.
    pub sgr: bool,
}

impl MouseMode {
    /// No mouse tracking, no SGR -- the initial/default state before a
    /// program ever requests mouse reporting.
    pub const OFF: MouseMode = MouseMode {
        tracking: MouseTracking::Off,
        sgr: false,
    };

    /// Packs into a single byte: bits `0..=2` hold [`MouseTracking::
    /// to_bits`], bit `3` holds `sgr`. Used by [`crate::TermSession`] to
    /// publish this through one `AtomicU8`.
    pub(crate) fn to_bits(self) -> u8 {
        self.tracking.to_bits() | ((self.sgr as u8) << 3)
    }

    /// Inverse of [`Self::to_bits`].
    pub(crate) fn from_bits(bits: u8) -> Self {
        MouseMode {
            tracking: MouseTracking::from_bits(bits & 0b0111),
            sgr: bits & 0b1000 != 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_round_trip_every_tracking_mode_with_and_without_sgr() {
        for tracking in [
            MouseTracking::Off,
            MouseTracking::X10,
            MouseTracking::Normal,
            MouseTracking::Button,
            MouseTracking::Any,
        ] {
            for sgr in [false, true] {
                let mode = MouseMode { tracking, sgr };
                assert_eq!(MouseMode::from_bits(mode.to_bits()), mode);
            }
        }
    }

    #[test]
    fn default_and_off_are_equivalent() {
        assert_eq!(MouseMode::default(), MouseMode::OFF);
        assert_eq!(MouseMode::OFF.to_bits(), 0);
    }

    #[test]
    fn is_active_is_false_only_for_off() {
        assert!(!MouseTracking::Off.is_active());
        assert!(MouseTracking::X10.is_active());
        assert!(MouseTracking::Normal.is_active());
        assert!(MouseTracking::Button.is_active());
        assert!(MouseTracking::Any.is_active());
    }

    #[test]
    fn reports_motion_only_for_button_and_any() {
        assert!(!MouseTracking::Off.reports_motion());
        assert!(!MouseTracking::X10.reports_motion());
        assert!(!MouseTracking::Normal.reports_motion());
        assert!(MouseTracking::Button.reports_motion());
        assert!(MouseTracking::Any.reports_motion());
    }

    #[test]
    fn reports_hover_motion_only_for_any() {
        assert!(!MouseTracking::Button.reports_hover_motion());
        assert!(MouseTracking::Any.reports_hover_motion());
    }

    #[test]
    fn from_bits_out_of_range_tracking_falls_back_to_off() {
        // Bits 0b101..=0b111 (5, 6, 7) are not a valid tracking discriminant.
        assert_eq!(MouseTracking::from_bits(5), MouseTracking::Off);
        assert_eq!(MouseTracking::from_bits(7), MouseTracking::Off);
    }
}
