//! UI-chrome i18n (wave 6f): ja/en locale tables compiled in from
//! `locales/{ja,en}.yml` (`rust_i18n::i18n!`, invoked once in `main.rs`) and
//! looked up everywhere else via the `t!()` macro (`use rust_i18n::t;` per
//! file, per rust-i18n's own convention -- no `#[macro_use]` needed).
//!
//! ## Scope
//!
//! Menu bar, sidebar, settings overlay, Git pane/tiles, the Task "…" menu
//! and its delete-confirmation dialog, the Swift importer's banner, and the
//! empty-workspace placeholders -- see the module docs of each of those for
//! their own `t!()` call sites. Deliberately **not** localized: terminal
//! content itself, Ghostty's own config, the `labolabo` control CLI's
//! output (`docs/control-protocol.md` already fixes it to plain English),
//! and developer-facing `eprintln!` warnings (stderr is not a user-facing
//! surface).
//!
//! ## [`LocaleSetting`] and effective-locale resolution
//!
//! Persisted as one more `appState` key (`TaskDatabase::locale`/
//! `set_locale`, mirroring `AppSettings`'s three existing settings keys --
//! see `settings.rs`'s module doc comment for that pattern). `Auto` (the
//! default -- a fresh database has never set this key) resolves to the OS
//! locale (`detect_os_locale`, `sys_locale::get_locale()`): `ja` if it
//! starts with `"ja"` (`"ja-JP"`, `"ja"`, ...), `en` otherwise -- this port
//! only ships two locales, so anything that isn't Japanese falls back to
//! English rather than a third undefined state.
//!
//! ## Live switching, not "reflects after restart"
//!
//! The task brief allows either ("変更は即時反映が難しければ「再起動後に
//! 反映」表記で可"), but live switching turned out to be the simpler of the
//! two here: every render function already calls `t!()` fresh on every
//! frame (ordinary `cx.notify()`-driven redraw, no caching), and gpui's
//! `App::set_menus` is a plain "replace the menu bar" call, not a one-shot
//! startup-only API -- so `LaboLaboApp::set_locale` (`app.rs`) just calls
//! `rust_i18n::set_locale`, rebuilds the menu bar
//! (`cx.set_menus(menus::app_menus(&locale))`), and `cx.notify()`s. No
//! "restart to apply" state machine needed.

use labolabo_core::{PaneKind, TaskDatabase};
use rust_i18n::t;

/// The settings screen's language choice (`plans/014`-style small enum,
/// mirrors `settings::AppSettings`'s other fields). `Auto` is the default --
/// see this module's doc comment for how it resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LocaleSetting {
    #[default]
    Auto,
    Ja,
    En,
}

impl LocaleSetting {
    /// `TaskDatabase`'s `appState` encoding -- plain ASCII tags, same
    /// "trivial to eyeball in the raw table" rationale as `bool_flag`'s
    /// `"1"`/`"0"` (`task_database.rs`).
    pub fn as_db_str(self) -> &'static str {
        match self {
            LocaleSetting::Auto => "auto",
            LocaleSetting::Ja => "ja",
            LocaleSetting::En => "en",
        }
    }

    /// Unknown/corrupt stored text degrades to `Auto` -- this crate's usual
    /// "unrecognized persisted data falls back to the default" posture
    /// (matches e.g. `TaskStatus::parse`), rather than erroring.
    pub fn from_db_str(value: Option<&str>) -> Self {
        match value {
            Some("ja") => LocaleSetting::Ja,
            Some("en") => LocaleSetting::En,
            _ => LocaleSetting::Auto,
        }
    }

    /// Resolves to the actual `rust_i18n` locale tag this setting means
    /// right now -- `Auto` re-detects the OS locale every call (cheap: one
    /// `sys_locale::get_locale()`), so this always reflects the current OS
    /// setting even if the user changes it without restarting LaboLabo.
    pub fn resolve(self) -> &'static str {
        match self {
            LocaleSetting::Ja => "ja",
            LocaleSetting::En => "en",
            LocaleSetting::Auto => detect_os_locale(),
        }
    }
}

/// `ja` if the OS's most-preferred locale (`sys_locale::get_locale`, a BCP
/// 47 tag like `"ja-JP"`) starts with `"ja"`; `en` for everything else,
/// including "couldn't detect" (`None`) -- this port ships exactly two
/// locales, so there is no meaningful third fallback to pick.
fn detect_os_locale() -> &'static str {
    match sys_locale::get_locale() {
        Some(tag) if tag.to_ascii_lowercase().starts_with("ja") => "ja",
        _ => "en",
    }
}

/// Reads the persisted language setting (`None`/corrupt -> `Auto`, same
/// "degrades to the default" contract every other `AppSettings` field
/// follows -- see `settings::AppSettings::load`).
pub fn load_locale_setting(db: &TaskDatabase) -> LocaleSetting {
    LocaleSetting::from_db_str(db.locale().ok().flatten().as_deref())
}

/// The localized counterpart of `labolabo_core::PaneKind::default_title`
/// (the core crate has no i18n dependency and keeps its unlocalized
/// Japanese defaults for the restore-a-title-less-persisted-pane path).
/// Used by `app.rs` everywhere a *new* pane is created, so a freshly opened
/// tab's title matches the UI language at creation time. A pane's title is
/// persisted data (round-trips through the Task's `TileLayout`), so titles
/// created under one language deliberately keep that language after a
/// switch -- same as a user-renamed title would.
pub fn default_pane_title(kind: PaneKind) -> String {
    match kind {
        PaneKind::Terminal => t!("pane.default_title.terminal").to_string(),
        PaneKind::Files => t!("pane.default_title.files").to_string(),
        PaneKind::Diff => t!("pane.default_title.diff").to_string(),
        PaneKind::Commits => t!("pane.default_title.commits").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_str_round_trips_through_from_db_str() {
        for setting in [LocaleSetting::Auto, LocaleSetting::Ja, LocaleSetting::En] {
            assert_eq!(
                LocaleSetting::from_db_str(Some(setting.as_db_str())),
                setting
            );
        }
    }

    #[test]
    fn unrecognized_or_absent_db_text_falls_back_to_auto() {
        assert_eq!(LocaleSetting::from_db_str(None), LocaleSetting::Auto);
        assert_eq!(LocaleSetting::from_db_str(Some("fr")), LocaleSetting::Auto);
        assert_eq!(LocaleSetting::from_db_str(Some("")), LocaleSetting::Auto);
    }

    #[test]
    fn ja_and_en_resolve_to_themselves_regardless_of_the_os_locale() {
        assert_eq!(LocaleSetting::Ja.resolve(), "ja");
        assert_eq!(LocaleSetting::En.resolve(), "en");
    }

    #[test]
    fn load_locale_setting_defaults_to_auto_on_a_fresh_database() {
        let db = TaskDatabase::open_in_memory().unwrap();
        assert_eq!(load_locale_setting(&db), LocaleSetting::Auto);
    }

    #[test]
    fn load_locale_setting_reflects_a_persisted_value() {
        let db = TaskDatabase::open_in_memory().unwrap();
        db.set_locale("ja").unwrap();
        assert_eq!(load_locale_setting(&db), LocaleSetting::Ja);
    }
}
