//! The boundary the native shells call. Its types mirror `launcher-core`'s so the core stays
//! free of FFI derives; a Rust shell (Linux) uses the core directly.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use launcher_core::{Found, PluginId};
use launcher_window::{Display, Placement, Rect, WindowAction};

uniffi::setup_scaffolding!();

#[derive(uniffi::Record)]
pub struct Item {
    pub title: String,
    pub subtitle: String,
    pub action: Action,
    pub detail: Vec<Section>,
    /// Handed back to `Launcher::ran` once the action has run.
    pub choice: Arc<Choice>,
}

#[derive(uniffi::Record)]
pub struct Section {
    pub title: String,
    pub text: String,
}

impl From<launcher_core::Section> for Section {
    fn from(section: launcher_core::Section) -> Self {
        Self {
            title: section.title,
            text: section.text,
        }
    }
}

#[derive(uniffi::Enum)]
pub enum Action {
    Copy { text: String },
    Open { path: String },
    Nothing,
}

/// Which plugin an item came from, and its action as the plugin made it: paths stay `PathBuf`s
/// rather than the lossy strings the shell sees.
#[derive(uniffi::Object)]
pub struct Choice {
    plugin: PluginId,
    action: launcher_core::Action,
}

impl From<Found> for Item {
    fn from(found: Found) -> Self {
        let action = match &found.item.action {
            launcher_core::Action::Copy(text) => Action::Copy { text: text.clone() },
            launcher_core::Action::Open(path) => Action::Open {
                path: path.to_string_lossy().into_owned(),
            },
            launcher_core::Action::Nothing => Action::Nothing,
        };
        Self {
            title: found.item.title,
            subtitle: found.item.subtitle,
            action,
            detail: found.item.detail.into_iter().map(Section::from).collect(),
            choice: Arc::new(Choice {
                plugin: found.plugin,
                action: found.item.action,
            }),
        }
    }
}

// Restated for UniFFI to derive its converters; the compiler checks they match the originals.
#[uniffi::remote(Record)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[uniffi::remote(Record)]
pub struct Display {
    pub frame: Rect,
    pub visible: Rect,
}

#[uniffi::remote(Record)]
pub struct Placement {
    pub frame: Rect,
    pub visible: Rect,
}

#[uniffi::remote(Enum)]
pub enum WindowAction {
    LeftHalf,
    RightHalf,
    TopHalf,
    BottomHalf,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    FirstThird,
    CentreThird,
    LastThird,
    Maximise,
    Centre,
    Restore,
    NextDisplay,
    PreviousDisplay,
}

#[derive(uniffi::Record)]
pub struct Hotkey {
    pub key: Key,
    pub control: bool,
    pub option: bool,
    pub command: bool,
    pub shift: bool,
}

impl From<launcher_core::Hotkey> for Hotkey {
    fn from(hotkey: launcher_core::Hotkey) -> Self {
        Self {
            key: hotkey.key.into(),
            control: hotkey.control,
            option: hotkey.option,
            command: hotkey.command,
            shift: hotkey.shift,
        }
    }
}

#[derive(uniffi::Enum)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Return,
    Delete,
    Space,
    /// UniFFI has no `char`.
    Character {
        character: String,
    },
}

impl From<launcher_core::Key> for Key {
    fn from(key: launcher_core::Key) -> Self {
        match key {
            launcher_core::Key::Left => Self::Left,
            launcher_core::Key::Right => Self::Right,
            launcher_core::Key::Up => Self::Up,
            launcher_core::Key::Down => Self::Down,
            launcher_core::Key::Return => Self::Return,
            launcher_core::Key::Delete => Self::Delete,
            launcher_core::Key::Space => Self::Space,
            launcher_core::Key::Character(character) => Self::Character {
                character: character.to_string(),
            },
        }
    }
}

/// A global hotkey and what it does, for the shell to register.
#[derive(uniffi::Record)]
pub struct HotkeyBinding {
    pub hotkey: Hotkey,
    /// `None` for the hotkey that opens the launcher.
    pub action: Option<WindowAction>,
    /// The setting that binds it and its text, as the config file writes them, for messages:
    /// `window.left_half = "ctrl+alt+left"`.
    pub setting: String,
}

/// The settings a shell applies itself: its global hotkeys and menu bar item.
#[derive(uniffi::Record)]
pub struct Settings {
    pub bindings: Vec<HotkeyBinding>,
    pub menu_bar: bool,
    /// Why the config file can't be used, naming it; the settings are then the ones from before,
    /// or the defaults.
    pub problem: Option<String>,
}

/// Runs `config` commands, such as `config init`, before the shell starts its UI: `None` when
/// `arguments` aren't one, else the exit status.
#[uniffi::export]
pub fn config_command(arguments: Vec<String>) -> Option<i32> {
    launcher_config::run_command(&arguments)
}

#[derive(Debug, uniffi::Error)]
pub enum SettingsFileError {
    Unavailable { reason: String },
}

impl fmt::Display for SettingsFileError {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let Self::Unavailable { reason } = self;
        formatter.write_str(reason)
    }
}

/// The config file's path, for the shell to open in an editor. Without a file, writes one first
/// with every setting commented out, as `config init` does.
#[uniffi::export]
pub fn settings_file() -> Result<String, SettingsFileError> {
    let path = launcher_config::file_to_edit()
        .map_err(|reason| SettingsFileError::Unavailable { reason })?;
    path.into_os_string()
        .into_string()
        .map_err(|path| SettingsFileError::Unavailable {
            reason: format!("{} isn't UTF-8, so it can't be opened", path.display()),
        })
}

/// Where `action` puts `window`, all in top-left-origin global coordinates; `None` for `Restore`,
/// for a display move with only one display, and without displays.
#[uniffi::export]
pub fn window_placement(
    action: WindowAction,
    window: Rect,
    displays: Vec<Display>,
) -> Option<Placement> {
    action.place(window, &displays)
}

/// Where `Restore` puts `window` back to `original`, saved among the displays `saved`; `None` to
/// restore it exactly, as while its display is still where it was.
#[uniffi::export]
pub fn window_restore_placement(
    original: Rect,
    saved: Vec<Display>,
    window: Rect,
    displays: Vec<Display>,
) -> Option<Placement> {
    launcher_window::restore_placement(original, &saved, window, &displays)
}

/// `frame` moved as little as it takes to lie within `visible`, keeping its size.
#[uniffi::export]
pub fn window_moved_within(frame: Rect, visible: Rect) -> Rect {
    launcher_window::moved_within(frame, visible)
}

#[derive(uniffi::Object)]
pub struct Launcher(Mutex<launcher_plugins::Launcher>);

impl Launcher {
    /// `None` after a plugin panicked while holding the lock.
    fn lock(&self) -> Option<MutexGuard<'_, launcher_plugins::Launcher>> {
        let locked = self.0.lock().ok();
        if locked.is_none() {
            eprintln!("a plugin panicked; the launcher is unavailable");
        }
        locked
    }
}

impl From<&launcher_plugins::Launcher> for Settings {
    fn from(launcher: &launcher_plugins::Launcher) -> Self {
        Self {
            bindings: launcher
                .config()
                .hotkeys()
                .into_iter()
                .map(|setting| HotkeyBinding {
                    hotkey: setting.hotkey.into(),
                    action: setting.action,
                    setting: format!("{} = \"{}\"", setting.name, setting.hotkey),
                })
                .collect(),
            menu_bar: launcher.config().menu_bar,
            problem: launcher.problem().map(String::from),
        }
    }
}

/// The shell's side of `launcher_core::ResultsChanged`.
#[uniffi::export(callback_interface)]
pub trait ResultsListener: Send + Sync {
    /// Called from any thread when the results for the shown query may have changed.
    fn results_changed(&self);
}

#[uniffi::export(callback_interface)]
pub trait SettingsListener: Send + Sync {
    /// Called from another thread when the config file changed, for the shell to call
    /// `Launcher::reload_settings`.
    fn settings_changed(&self);
}

#[uniffi::export]
impl Launcher {
    /// Starts the plugins the config file turns on, and watches the file.
    #[uniffi::constructor]
    pub fn new(
        listener: Box<dyn ResultsListener>,
        settings_listener: Box<dyn SettingsListener>,
    ) -> Arc<Self> {
        let listener: Arc<dyn ResultsListener> = Arc::from(listener);
        Arc::new(Self(Mutex::new(launcher_plugins::Launcher::start(
            Arc::new(move || listener.results_changed()),
            move || settings_listener.settings_changed(),
        ))))
    }

    pub fn settings(&self) -> Settings {
        match self.lock() {
            Some(launcher) => Settings::from(&*launcher),
            None => Settings {
                bindings: Vec::new(),
                menu_bar: launcher_config::Config::default().menu_bar,
                problem: Some("a plugin crashed; relaunch to get the settings back".into()),
            },
        }
    }

    /// Reads the config file again: `None` when nothing changed.
    pub fn reload_settings(&self) -> Option<Settings> {
        let mut launcher = self.lock()?;
        launcher.reload().then(|| Settings::from(&*launcher))
    }

    /// Called on every keystroke from the UI thread, so plugins must answer well inside a frame.
    pub fn query(&self, query: String) -> Vec<Item> {
        let Some(mut launcher) = self.lock() else {
            return Vec::new();
        };
        launcher
            .plugins()
            .query(&query)
            .into_iter()
            .map(Item::from)
            .collect()
    }

    /// The shell ran an item's action successfully.
    pub fn ran(&self, choice: Arc<Choice>) {
        if let Some(mut launcher) = self.lock() {
            launcher.plugins().ran(choice.plugin, &choice.action);
        }
    }
}

#[cfg(test)]
mod tests {
    use launcher_calc::Calc;
    use launcher_config::Config;
    use launcher_core::{Plugin, Plugins};

    use super::*;

    struct Pending;

    impl Plugin for Pending {
        fn query(&mut self, _query: &str) -> Vec<launcher_core::Item> {
            vec![launcher_core::Item {
                title: "Loading…".into(),
                subtitle: String::new(),
                action: launcher_core::Action::Nothing,
                detail: Vec::new(),
            }]
        }
    }

    /// Without the config file.
    fn launcher(plugins: Plugins) -> Launcher {
        Launcher(Mutex::new(launcher_plugins::Launcher::with(
            plugins,
            Config::default(),
            None,
            Arc::new(|| {}),
        )))
    }

    #[test]
    fn settings_name_each_hotkey_as_the_file_does() {
        let settings = launcher(Plugins::new(Vec::new())).settings();
        assert_eq!(settings.bindings.len(), Config::default().window.len() + 1);
        let named: Vec<(&str, Option<WindowAction>)> = settings
            .bindings
            .iter()
            .take(2)
            .map(|binding| (binding.setting.as_str(), binding.action))
            .collect();
        assert_eq!(
            named,
            [
                ("hotkey = \"alt+space\"", None),
                (
                    "window.left_half = \"ctrl+alt+left\"",
                    Some(WindowAction::LeftHalf)
                ),
            ]
        );
        assert!(settings.menu_bar);
        assert_eq!(settings.problem, None);
    }

    #[test]
    fn query_converts_items_without_an_action() {
        let launcher = launcher(Plugins::new(vec![Box::new(Pending)]));
        let items = launcher.query("anything".into());
        assert!(matches!(
            items.as_slice(),
            [Item {
                action: Action::Nothing,
                ..
            }]
        ));
    }

    #[test]
    fn query_converts_items() -> Result<(), Box<dyn std::error::Error>> {
        // Only the calculator, so installed apps can't add results.
        let launcher = launcher(Plugins::new(vec![Box::new(Calc::new()?)]));
        let items = launcher.query("2 + 3".into());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "5");
        assert!(matches!(&items[0].action, Action::Copy { text } if text == "5"));
        assert_eq!(
            items[0].choice.action,
            launcher_core::Action::Copy("5".into())
        );
        assert_eq!(
            items[0]
                .detail
                .iter()
                .map(|section| (section.title.as_str(), section.text.as_str()))
                .collect::<Vec<_>>(),
            [("Input", "2 + 3")]
        );
        Ok(())
    }
}
