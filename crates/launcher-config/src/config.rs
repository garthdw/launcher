//! Settings from `~/.config/launcher/config.toml`: the hotkeys and which plugins run. Every shell
//! reads them through here, so the file means the same on every platform.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use launcher_core::{Hotkey, Key, config_directory, log};
use launcher_window::{Binding, DEFAULT_BINDINGS, WindowAction};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Deserialize;
use serde::de::{self, Deserializer};

/// What `config init` writes: every setting commented out at its default, so the file overrides
/// only what the user uncomments. Settings are commented with a bare `#`, prose with `# `.
pub const DEFAULT_FILE: &str = include_str!("default.toml");
/// Editors save in bursts (a temporary file, then a rename over the old one); reload once quiet.
const SETTLE: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Opens the launcher.
    #[serde(deserialize_with = "hotkey")]
    pub hotkey: Hotkey,
    /// Shows an item in the menu bar to open the launcher or the config file, reload settings,
    /// launch at login and quit. Off by default, since showing it takes about 2 MB.
    pub menu_bar: bool,
    pub plugins: EnabledPlugins,
    /// The window actions with a hotkey, in `DEFAULT_BINDINGS`' order.
    #[serde(deserialize_with = "window_bindings")]
    pub window: Vec<Binding>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: Hotkey {
                key: Key::Space,
                control: false,
                option: true,
                command: false,
                shift: false,
            },
            menu_bar: false,
            plugins: EnabledPlugins::default(),
            window: DEFAULT_BINDINGS.to_vec(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(default, deny_unknown_fields)]
pub struct EnabledPlugins {
    pub calculator: bool,
    pub jwt: bool,
    pub apps: bool,
}

impl Default for EnabledPlugins {
    fn default() -> Self {
        Self {
            calculator: true,
            jwt: true,
            apps: true,
        }
    }
}

fn hotkey<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Hotkey, D::Error> {
    String::deserialize(deserializer)?
        .parse()
        .map_err(de::Error::custom)
}

/// The `[window]` table: hotkeys, or `none`, by action name, over the defaults.
fn window_bindings<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Binding>, D::Error> {
    let mut chosen = HashMap::<ActionName, WindowHotkey>::deserialize(deserializer)?;
    Ok(DEFAULT_BINDINGS
        .iter()
        .filter_map(|default| {
            let hotkey = chosen
                .remove(&ActionName(default.action))
                .map_or(Some(default.hotkey), |chosen| chosen.0)?;
            Some(Binding {
                hotkey,
                action: default.action,
            })
        })
        .collect())
}

/// Read on its own rather than as part of the whole table, so a mistake points at its line.
#[derive(PartialEq, Eq, Hash)]
struct ActionName(WindowAction);

impl<'de> Deserialize<'de> for ActionName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        let actions = DEFAULT_BINDINGS.iter().map(|binding| binding.action);
        if let Some(action) = actions.clone().find(|action| action.name() == name) {
            return Ok(Self(action));
        }
        let names: Vec<&str> = actions.map(WindowAction::name).collect();
        Err(de::Error::custom(format!(
            "unknown window action `{name}`: use {}",
            names.join(", ")
        )))
    }
}

/// `None` for `none`. Read on its own, like `ActionName`.
struct WindowHotkey(Option<Hotkey>);

impl<'de> Deserialize<'de> for WindowHotkey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text == "none" {
            return Ok(Self(None));
        }
        text.parse()
            .map(|hotkey| Self(Some(hotkey)))
            .map_err(de::Error::custom)
    }
}

/// A hotkey and what it does, named as the config file names it, for a shell to register.
#[derive(Clone, Debug, PartialEq)]
pub struct HotkeySetting {
    /// Such as `window.left_half`, for messages.
    pub name: String,
    pub hotkey: Hotkey,
    /// `None` for the hotkey that opens the launcher.
    pub action: Option<WindowAction>,
}

impl Config {
    /// The launcher hotkey, then the window actions' hotkeys.
    pub fn hotkeys(&self) -> Vec<HotkeySetting> {
        let window = self.window.iter().map(|binding| HotkeySetting {
            name: format!("window.{}", binding.action.name()),
            hotkey: binding.hotkey,
            action: Some(binding.action),
        });
        let launcher = HotkeySetting {
            name: "hotkey".into(),
            hotkey: self.hotkey,
            action: None,
        };
        [launcher].into_iter().chain(window).collect()
    }
}

/// Settings from a config file's text. A mistake rejects the whole file, so a half-read file
/// can't take hotkeys away.
pub fn parse(text: &str) -> Result<Config, String> {
    let config: Config = toml::from_str(text).map_err(|error| {
        // One line rather than toml's source excerpt, whose caret only lines up in a
        // monospaced font.
        let line = error
            .span()
            .and_then(|span| text.get(..span.start))
            .map(|before| before.matches('\n').count() + 1);
        match line {
            Some(line) => format!("line {line}: {}", error.message()),
            None => error.message().to_string(),
        }
    })?;
    let hotkeys = config.hotkeys();
    for (index, setting) in hotkeys.iter().enumerate() {
        if let Some(other) = hotkeys
            .iter()
            .skip(index + 1)
            .find(|other| other.hotkey == setting.hotkey)
        {
            return Err(format!(
                "{} and {} both use `{}`",
                setting.name, other.name, setting.hotkey
            ));
        }
    }
    Ok(config)
}

/// `~/.config/launcher/config.toml`.
pub fn path() -> Result<PathBuf, String> {
    config_directory()
        .map(|directory| directory.join("config.toml"))
        .ok_or_else(|| "no home folder, so no config file".into())
}

/// The settings in the config file, or the defaults without one. The error says why the file
/// can't be used, naming it.
pub fn load() -> Result<Config, String> {
    read(&path()?)
}

fn read(path: &Path) -> Result<Config, String> {
    match fs::read_to_string(path) {
        Ok(text) => parse(&text).map_err(|error| format!("{}: {error}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
        Err(error) => Err(format!("can't read {}: {error}", path.display())),
    }
}

/// Runs `config` commands, which every shell shares: the shell hands over its arguments, after
/// trying its own commands (macOS's `login`), before starting its UI. `None` when they aren't a
/// command, so the UI starts; else the exit status.
#[expect(
    clippy::disallowed_macros,
    reason = "a command's messages go to its terminal, not the log"
)]
pub fn run_command(arguments: &[String]) -> Option<i32> {
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let ["config", command @ ..] = arguments.as_slice() else {
        return None;
    };
    let path = match path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            return Some(1);
        }
    };
    if command != ["init"] {
        eprintln!(
            "usage: config init\n  writes every setting, commented out at its default, to {}",
            path.display()
        );
        return Some(64);
    }
    Some(match init(&path) {
        Ok(true) => {
            println!("wrote {}", path.display());
            0
        }
        Ok(false) => {
            eprintln!(
                "{} already exists; move it aside to write the defaults again",
                path.display()
            );
            1
        }
        Err(error) => {
            eprintln!("can't write {}: {error}", path.display());
            1
        }
    })
}

/// The config file, writing what `config init` does first when there's none, so an editor opens
/// on every setting rather than an empty file.
pub fn file_to_edit() -> Result<PathBuf, String> {
    let path = path()?;
    init(&path).map_err(|error| format!("can't write {}: {error}", path.display()))?;
    Ok(path)
}

/// Writes `DEFAULT_FILE` to `path`; `false` when a file is already there, left alone.
fn init(path: &Path) -> io::Result<bool> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => match file.write_all(DEFAULT_FILE.as_bytes()) {
            Ok(()) => Ok(true),
            Err(error) => {
                // Left in place, a partial file would pass for the user's own from then on.
                if let Err(removing) = fs::remove_file(path) {
                    log!("can't remove the partial {}: {removing}", path.display());
                }
                Err(error)
            }
        },
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error),
    }
}

/// Calls `changed` on another thread once a burst of changes to the file at `path` settles.
/// Watches its folder, creating it if need be, so a file created later or saved by renaming
/// another over it is seen too. When the file is a symlink, as dotfile managers make, its
/// target's folder is watched as well. Watching stops when the returned watcher drops.
pub fn watch(
    path: &Path,
    changed: impl Fn() + Send + 'static,
) -> notify::Result<RecommendedWatcher> {
    let (Some(directory), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(notify::Error::generic("not a file path").add_path(path.into()));
    };
    fs::create_dir_all(directory).map_err(notify::Error::io)?;
    let target = fs::canonicalize(path)
        .ok()
        .filter(|target| target.parent() != fs::canonicalize(directory).ok().as_deref());
    let target_directory = target.as_deref().and_then(Path::parent);
    let names: Vec<_> = [Some(name), target.as_deref().and_then(Path::file_name)]
        .into_iter()
        .flatten()
        .map(|name| name.to_os_string())
        .collect();
    let (sender, receiver) = mpsc::channel();
    let mut watcher =
        notify::recommended_watcher(move |result: notify::Result<notify::Event>| match result {
            Ok(event)
                if !event.kind.is_access()
                    && event
                        .paths
                        .iter()
                        .filter_map(|path| path.file_name())
                        .any(|name| names.iter().any(|watched| watched == name)) =>
            {
                if sender.send(()).is_err() {
                    log!("the config reload thread stopped; ignoring changes");
                }
            }
            Ok(_) => {}
            Err(error) => log!("config watcher: {error}"),
        })?;
    watcher.watch(directory, RecursiveMode::NonRecursive)?;
    if let Some(target_directory) = target_directory {
        watcher.watch(target_directory, RecursiveMode::NonRecursive)?;
    }
    thread::spawn(move || {
        while receiver.recv().is_ok() {
            while receiver.recv_timeout(SETTLE).is_ok() {}
            changed();
        }
    });
    Ok(watcher)
}

#[cfg(test)]
mod tests {

    use serde_json::Value;

    use super::*;

    fn parsed(text: &str) -> Hotkey {
        match text.parse() {
            Ok(hotkey) => hotkey,
            Err(error) => panic!("{error}"),
        }
    }

    fn error(text: &str) -> String {
        parse(text).err().unwrap_or_default()
    }

    #[test]
    fn an_empty_file_is_the_defaults() {
        assert_eq!(parse(""), Ok(Config::default()));
        assert_eq!(Config::default().hotkey, parsed("alt+space"));
    }

    /// `DEFAULT_FILE` with every setting uncommented, as a user would.
    fn uncommented_default_file() -> String {
        DEFAULT_FILE
            .lines()
            .map(|line| match line.strip_prefix('#') {
                Some(setting) if setting.starts_with(|first: char| first.is_ascii_alphabetic()) => {
                    setting
                }
                _ => line,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_default_file_is_the_defaults_commented_or_not() {
        assert_eq!(parse(DEFAULT_FILE), Ok(Config::default()));
        let uncommented = uncommented_default_file();
        assert!(uncommented.contains("\nhotkey = \"alt+space\"\n"));
        assert_eq!(parse(&uncommented), Ok(Config::default()));
    }

    #[test]
    fn settings_override_the_defaults_and_none_unbinds() {
        let config = parse(
            r#"
            hotkey = "cmd+space"
            menu_bar = true
            [plugins]
            jwt = false
            [window]
            left_half = "ctrl+alt+shift+left"
            restore = "none"
            "#,
        );
        let Ok(config) = config else {
            panic!("{config:?}");
        };
        assert_eq!(config.hotkey, parsed("cmd+space"));
        assert!(config.menu_bar);
        assert_eq!(
            config.plugins,
            EnabledPlugins {
                calculator: true,
                jwt: false,
                apps: true,
            }
        );
        assert_eq!(
            config.window.first(),
            Some(&Binding {
                hotkey: parsed("ctrl+alt+shift+left"),
                action: WindowAction::LeftHalf,
            })
        );
        assert_eq!(config.window.len(), DEFAULT_BINDINGS.len() - 1);
        assert!(
            config
                .window
                .iter()
                .all(|binding| binding.action != WindowAction::Restore)
        );
    }

    #[test]
    fn mistakes_name_the_line_and_the_setting() {
        assert_eq!(
            error("[window]\nleft_half = \"ctrl+alt+lft\"\n"),
            "line 2: unknown key `lft` in `ctrl+alt+lft`: use a letter, a digit or left, right, \
             up, down, return, delete, space"
        );
        assert_eq!(
            error("\nhotkey = \"shift+space\"\n"),
            "line 2: `shift+space` needs ctrl, alt or cmd, so it doesn't take over typing"
        );
        let message = error("[window]\nleft_halve = \"ctrl+alt+left\"\n");
        assert!(
            message.starts_with(
                "line 2: unknown window action `left_halve`: use left_half, right_half, "
            ),
            "{message}"
        );
        assert_eq!(
            error("[plugins]\ncalculater = false\n"),
            "line 2: unknown field `calculater`, expected one of `calculator`, `jwt`, `apps`"
        );
        let message = error("hotkey = 3");
        assert!(message.starts_with("line 1: invalid type"), "{message}");
        let message = error("hotkey = \"alt+space");
        assert!(message.starts_with("line 1: "), "{message}");
    }

    #[test]
    fn a_hotkey_bound_twice_is_a_mistake() {
        assert_eq!(
            error("[window]\nleft_half = \"ctrl+alt+c\"\n"),
            "window.left_half and window.centre both use `ctrl+alt+c`"
        );
        assert_eq!(
            error("hotkey = \"ctrl+alt+left\"\n"),
            "hotkey and window.left_half both use `ctrl+alt+left`"
        );
        assert!(parse("[window]\nleft_half = \"ctrl+alt+c\"\ncentre = \"none\"\n").is_ok());
    }

    #[test]
    fn read_falls_back_to_the_defaults_only_without_a_file() -> io::Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("config.toml");
        assert_eq!(read(&path), Ok(Config::default()));
        fs::write(&path, "hotkey = \"nonsense\"")?;
        let message = read(&path).err().unwrap_or_default();
        assert!(
            message.starts_with(&format!("{}: ", path.display())),
            "{message}"
        );
        Ok(())
    }

    #[test]
    fn init_writes_the_defaults_once() -> io::Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("launcher/config.toml");
        assert!(init(&path)?);
        assert_eq!(fs::read_to_string(&path)?, DEFAULT_FILE);
        fs::write(&path, "hotkey = \"cmd+space\"")?;
        assert!(!init(&path)?);
        assert_eq!(fs::read_to_string(&path)?, "hotkey = \"cmd+space\"");
        Ok(())
    }

    #[test]
    fn a_file_in_place_of_the_folder_is_no_config_file() -> io::Result<()> {
        let folder = tempfile::tempdir()?;
        fs::write(folder.path().join("launcher"), "")?;
        assert!(init(&folder.path().join("launcher/config.toml")).is_err());
        Ok(())
    }

    #[test]
    fn other_arguments_start_the_app() {
        assert_eq!(run_command(&[]), None);
        assert_eq!(run_command(&["-NSDocumentRevisionsDebugMode".into()]), None);
        assert_eq!(run_command(&["config".into()]), Some(64));
        assert_eq!(run_command(&["config".into(), "nit".into()]), Some(64));
    }

    #[test]
    fn watch_reports_saves_including_renames_over_the_file()
    -> Result<(), Box<dyn std::error::Error>> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("launcher/config.toml");
        let (sender, receiver) = mpsc::channel();
        let _watcher = watch(&path, move || {
            if sender.send(()).is_err() {
                log!("the test stopped listening");
            }
        })?;
        let wait = Duration::from_secs(5);

        fs::write(&path, "hotkey = \"cmd+space\"")?;
        receiver.recv_timeout(wait)?;

        let temporary = path.with_file_name(".config.toml.swp");
        fs::write(&temporary, "hotkey = \"ctrl+space\"")?;
        fs::rename(&temporary, &path)?;
        receiver.recv_timeout(wait)?;

        fs::write(path.with_file_name("frecency"), "")?;
        assert!(receiver.recv_timeout(Duration::from_secs(1)).is_err());
        Ok(())
    }

    #[test]
    fn watch_follows_a_symlinked_file_to_its_target() -> Result<(), Box<dyn std::error::Error>> {
        let folder = tempfile::tempdir()?;
        let dotfiles = folder.path().join("dotfiles");
        fs::create_dir_all(&dotfiles)?;
        let target = dotfiles.join("launcher.toml");
        fs::write(&target, "")?;
        let path = folder.path().join("launcher/config.toml");
        fs::create_dir_all(folder.path().join("launcher"))?;
        std::os::unix::fs::symlink(&target, &path)?;
        let (sender, receiver) = mpsc::channel();
        let _watcher = watch(&path, move || {
            if sender.send(()).is_err() {
                log!("the test stopped listening");
            }
        })?;
        // Events from making the link can arrive late; only the save counts.
        while receiver.recv_timeout(Duration::from_secs(1)).is_ok() {}
        fs::write(&target, "hotkey = \"cmd+space\"")?;
        receiver.recv_timeout(Duration::from_secs(5))?;
        Ok(())
    }

    /// The schema editors read, kept in step with the settings.
    fn schema() -> Result<Value, Box<dyn std::error::Error>> {
        Ok(serde_json::from_str(include_str!("../config.schema.json"))?)
    }

    /// Each setting's default as the schema gives it, nested as in the file.
    fn schema_defaults(schema: &Value) -> Value {
        match schema["properties"].as_object() {
            Some(properties) => properties
                .iter()
                .map(|(name, property)| (name.clone(), schema_defaults(property)))
                .collect(),
            None => schema["default"].clone(),
        }
    }

    #[test]
    fn the_schema_and_default_file_cover_every_setting() -> Result<(), Box<dyn std::error::Error>> {
        let config = Config::default();
        let window: serde_json::Map<String, Value> = DEFAULT_BINDINGS
            .iter()
            .map(|binding| {
                (
                    binding.action.name().into(),
                    binding.hotkey.to_string().into(),
                )
            })
            .collect();
        let expected = serde_json::json!({
            "hotkey": config.hotkey.to_string(),
            "menu_bar": config.menu_bar,
            "plugins": serde_json::to_value(config.plugins)?,
            "window": window,
        });
        let schema = schema()?;
        assert_eq!(schema_defaults(&schema), expected);
        let file: toml::Table = toml::from_str(&uncommented_default_file())?;
        assert_eq!(serde_json::to_value(file)?, expected);
        // Maps compare regardless of order, but editors list the schema's in its order.
        let window_order: Vec<&String> = schema["properties"]["window"]["properties"]
            .as_object()
            .map(|properties| properties.keys().collect())
            .unwrap_or_default();
        assert_eq!(window_order, window.keys().collect::<Vec<_>>());
        Ok(())
    }

    #[test]
    fn the_schema_pattern_names_every_key_and_modifier() -> Result<(), Box<dyn std::error::Error>> {
        let keys: Vec<String> = Key::NAMED.iter().map(Key::to_string).collect();
        let pattern = format!(
            "^(({})\\+)+({}|[a-z0-9])$",
            Hotkey::MODIFIERS.join("|"),
            keys.join("|")
        );
        assert_eq!(schema()?["$defs"]["hotkey"]["pattern"], pattern);
        Ok(())
    }

    #[test]
    fn the_default_file_points_editors_at_the_schema() {
        assert!(DEFAULT_FILE.starts_with(
            "#:schema https://raw.githubusercontent.com/garthdw/launcher/main/crates/launcher-config/config.schema.json\n"
        ));
    }
}
