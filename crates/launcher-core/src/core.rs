use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub title: String,
    /// What the item is, such as its source. The shell adds the keys that act on it.
    pub subtitle: String,
    pub action: Action,
    /// Shown on request in a scrolling, monospaced view; empty for items without one.
    pub detail: Vec<Section>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub title: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Copy(String),
    Open(PathBuf),
    /// The item only says something, such as that work is under way; the shell draws it muted.
    Nothing,
}

/// A global key combination, named by key position on a US layout, which the shell registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub key: Key,
    pub control: bool,
    pub option: bool,
    pub command: bool,
    pub shift: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Return,
    Delete,
    Space,
    /// A letter or digit, in lower case.
    Character(char),
}

impl Key {
    /// The keys with names in a hotkey's text; letters and digits stand for themselves.
    pub const NAMED: [Self; 7] = [
        Self::Left,
        Self::Right,
        Self::Up,
        Self::Down,
        Self::Return,
        Self::Delete,
        Self::Space,
    ];
}

impl fmt::Display for Key {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let name = match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
            Self::Return => "return",
            Self::Delete => "delete",
            Self::Space => "space",
            Self::Character(character) => return write!(formatter, "{character}"),
        };
        formatter.write_str(name)
    }
}

impl Hotkey {
    /// The modifiers a hotkey's text may name; `opt` is another name for `alt`.
    pub const MODIFIERS: [&str; 5] = ["ctrl", "alt", "opt", "cmd", "shift"];
}

/// Modifiers then a key, joined by `+`: `ctrl+alt+left`, `cmd+shift+k`. A hotkey needs ctrl, alt
/// or cmd, so it can't take over typing.
impl FromStr for Hotkey {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut key_then_modifiers = text.rsplit('+');
        let key_name = key_then_modifiers.next().unwrap_or_default();
        let key = parse_key(key_name).ok_or_else(|| {
            if key_name.is_empty() || Self::MODIFIERS.contains(&key_name) {
                format!("`{text}` has no key after its modifiers")
            } else {
                let named: Vec<String> = Key::NAMED.iter().map(Key::to_string).collect();
                format!(
                    "unknown key `{key_name}` in `{text}`: use a letter, a digit or {}",
                    named.join(", ")
                )
            }
        })?;
        let mut hotkey = Self {
            key,
            control: false,
            option: false,
            command: false,
            shift: false,
        };
        for modifier in key_then_modifiers {
            let held = match modifier {
                "ctrl" => &mut hotkey.control,
                "alt" | "opt" => &mut hotkey.option,
                "cmd" => &mut hotkey.command,
                "shift" => &mut hotkey.shift,
                _ => {
                    return Err(format!(
                        "unknown modifier `{modifier}` in `{text}`: use {}",
                        Self::MODIFIERS.join(", ")
                    ));
                }
            };
            if *held {
                return Err(format!("`{text}` names a modifier twice"));
            }
            *held = true;
        }
        if !(hotkey.control || hotkey.option || hotkey.command) {
            return Err(format!(
                "`{text}` needs ctrl, alt or cmd, so it doesn't take over typing"
            ));
        }
        Ok(hotkey)
    }
}

fn parse_key(name: &str) -> Option<Key> {
    let mut characters = name.chars();
    match (characters.next(), characters.next()) {
        (Some(character), None) if character.is_ascii_lowercase() || character.is_ascii_digit() => {
            Some(Key::Character(character))
        }
        _ => Key::NAMED.into_iter().find(|key| key.to_string() == name),
    }
}

/// As `FromStr` reads it, with the modifiers in the order macOS menus show them.
impl fmt::Display for Hotkey {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let modifiers = [
            (self.control, "ctrl"),
            (self.option, "alt"),
            (self.shift, "shift"),
            (self.command, "cmd"),
        ];
        for (_, name) in modifiers.into_iter().filter(|(held, _)| *held) {
            write!(formatter, "{name}+")?;
        }
        write!(formatter, "{}", self.key)
    }
}

/// Where settings and state live: `~/.config/launcher` on every platform.
pub fn config_directory() -> Option<PathBuf> {
    std::env::home_dir().map(|home| home.join(".config/launcher"))
}

/// Where downloads that can be fetched again live: the platform's cache folder, which backups
/// skip. On macOS it is named for the bundle identifier, as `scripts/bundle` sets it.
pub fn cache_directory() -> Option<PathBuf> {
    let home = std::env::home_dir()?;
    Some(if cfg!(target_os = "macos") {
        home.join("Library/Caches/dev.dewet.launcher")
    } else {
        home.join(".cache/launcher")
    })
}

/// Writes a line to the log, which is stderr. Unlike `eprintln!`, a failed write doesn't panic:
/// the shell may point stderr at a file, which a full disk refuses.
#[macro_export]
macro_rules! log {
    ($($argument:tt)*) => {
        $crate::write_log(format_args!($($argument)*))
    };
}

#[doc(hidden)]
pub fn write_log(line: fmt::Arguments) {
    use std::io::Write;
    if let Err(_error) = writeln!(std::io::stderr(), "{line}") {
        // There's nowhere left to report that the log can't be written.
    }
}

/// Called from a plugin's background work when it may have changed the plugin's results, so the
/// shell asks again for the query it shows. Any thread may call it.
pub type ResultsChanged = Arc<dyn Fn() + Send + Sync>;

pub trait Plugin {
    /// Runs on the UI thread for every keystroke, so it must stay well under a frame.
    fn query(&mut self, query: &str) -> Vec<Item>;

    /// The shell ran `action` from one of this plugin's items.
    fn ran(&mut self, _action: &Action) {}
}

/// Which plugin produced an item. Only `Plugins::query` makes one, so it always names a plugin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PluginId(usize);

pub struct Found {
    pub plugin: PluginId,
    pub item: Item,
}

/// The plugins a shell runs, asked in order. A place can be empty, for a plugin that's off, so
/// turning one on or off leaves the others' ids, and the items they made, pointing at them.
pub struct Plugins(Vec<Option<Box<dyn Plugin + Send>>>);

impl Plugins {
    pub fn new(plugins: Vec<Box<dyn Plugin + Send>>) -> Self {
        Self(plugins.into_iter().map(Some).collect())
    }

    /// `count` empty places, for `set` to fill.
    pub fn empty(count: usize) -> Self {
        Self(std::iter::repeat_with(|| None).take(count).collect())
    }

    /// Puts `plugin`, or nothing, in place `place`, dropping the plugin there.
    pub fn set(&mut self, place: usize, plugin: Option<Box<dyn Plugin + Send>>) {
        match self.0.get_mut(place) {
            Some(slot) => *slot = plugin,
            None => log!("no place {place} for a plugin"),
        }
    }

    /// Runs on the UI thread for every keystroke, so every plugin must stay well under a frame.
    pub fn query(&mut self, query: &str) -> Vec<Found> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }
        self.0
            .iter_mut()
            .enumerate()
            .filter_map(|(index, plugin)| Some((index, plugin.as_mut()?)))
            .flat_map(|(index, plugin)| {
                plugin.query(query).into_iter().map(move |item| Found {
                    plugin: PluginId(index),
                    item,
                })
            })
            .collect()
    }

    /// The shell ran `action` from an item `plugin` produced; only that plugin hears of it.
    pub fn ran(&mut self, plugin: PluginId, action: &Action) {
        match self.0.get_mut(plugin.0).and_then(Option::as_mut) {
            Some(plugin) => plugin.ran(action),
            // Not the action: a copy's text can be a token's claims, and the log is kept on disk.
            None => log!("no plugin {} to tell it ran an action", plugin.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    type Ran = Arc<Mutex<Vec<Action>>>;

    struct Recorder {
        title: &'static str,
        ran: Ran,
    }

    impl Plugin for Recorder {
        fn query(&mut self, query: &str) -> Vec<Item> {
            vec![Item {
                title: format!("{} {query}", self.title),
                subtitle: String::new(),
                action: Action::Copy(self.title.into()),
                detail: Vec::new(),
            }]
        }

        fn ran(&mut self, action: &Action) {
            if let Ok(mut ran) = self.ran.lock() {
                ran.push(action.clone());
            }
        }
    }

    fn recorders() -> (Plugins, Ran, Ran) {
        let first = Arc::new(Mutex::new(Vec::new()));
        let second = Arc::new(Mutex::new(Vec::new()));
        let plugins = Plugins::new(vec![
            Box::new(Recorder {
                title: "first",
                ran: first.clone(),
            }),
            Box::new(Recorder {
                title: "second",
                ran: second.clone(),
            }),
        ]);
        (plugins, first, second)
    }

    #[test]
    fn query_asks_every_plugin_with_the_trimmed_query() {
        let (mut plugins, _, _) = recorders();
        let titles: Vec<String> = plugins
            .query("  2 + 3 ")
            .into_iter()
            .map(|found| found.item.title)
            .collect();
        assert_eq!(titles, ["first 2 + 3", "second 2 + 3"]);
        assert!(plugins.query("   ").is_empty());
    }

    #[test]
    fn ran_reaches_only_the_producing_plugin() {
        let (mut plugins, first, second) = recorders();
        let found = plugins.query("anything");
        let Some(chosen) = found.get(1) else {
            panic!("expected two items, got {}", found.len());
        };
        plugins.ran(chosen.plugin, &chosen.item.action);
        assert!(first.lock().is_ok_and(|ran| ran.is_empty()));
        assert!(
            second
                .lock()
                .is_ok_and(|ran| *ran == [Action::Copy("second".into())])
        );
    }

    #[test]
    fn emptying_a_place_keeps_the_others_ids() {
        let (mut plugins, _, second) = recorders();
        let found = plugins.query("anything");
        plugins.set(0, None);
        let titles: Vec<String> = plugins
            .query("again")
            .into_iter()
            .map(|found| found.item.title)
            .collect();
        assert_eq!(titles, ["second again"]);
        for chosen in &found {
            plugins.ran(chosen.plugin, &chosen.item.action);
        }
        assert!(
            second
                .lock()
                .is_ok_and(|ran| *ran == [Action::Copy("second".into())])
        );
    }

    fn hotkey(key: Key, modifiers: &str) -> Hotkey {
        Hotkey {
            key,
            control: modifiers.contains('⌃'),
            option: modifiers.contains('⌥'),
            command: modifiers.contains('⌘'),
            shift: modifiers.contains('⇧'),
        }
    }

    #[test]
    fn hotkeys_parse_modifiers_in_any_order() {
        assert_eq!("ctrl+alt+left".parse(), Ok(hotkey(Key::Left, "⌃⌥")));
        assert_eq!("cmd+opt+ctrl+right".parse(), Ok(hotkey(Key::Right, "⌃⌥⌘")));
        assert_eq!("shift+cmd+k".parse(), Ok(hotkey(Key::Character('k'), "⇧⌘")));
        assert_eq!("alt+space".parse(), Ok(hotkey(Key::Space, "⌥")));
        assert_eq!("ctrl+alt+7".parse(), Ok(hotkey(Key::Character('7'), "⌃⌥")));
    }

    #[test]
    fn every_named_key_and_modifier_parses() {
        for key in Key::NAMED {
            assert_eq!(format!("ctrl+{key}").parse(), Ok(hotkey(key, "⌃")));
        }
        // Shift alone takes over typing; `hotkey_errors_say_what_is_wrong` covers it.
        for modifier in Hotkey::MODIFIERS
            .iter()
            .filter(|&&modifier| modifier != "shift")
        {
            let modified = format!("{modifier}+x").parse::<Hotkey>();
            assert!(modified.is_ok(), "{modifier}: {modified:?}");
        }
    }

    #[test]
    fn hotkeys_print_as_they_parse() {
        for text in [
            "ctrl+alt+left",
            "ctrl+alt+shift+cmd+return",
            "alt+space",
            "cmd+9",
        ] {
            assert_eq!(
                text.parse::<Hotkey>().map(|hotkey| hotkey.to_string()),
                Ok(text.into())
            );
        }
        assert_eq!(
            "opt+cmd+ctrl+d"
                .parse::<Hotkey>()
                .map(|hotkey| hotkey.to_string()),
            Ok("ctrl+alt+cmd+d".into())
        );
    }

    #[test]
    fn hotkey_errors_say_what_is_wrong() {
        let error = |text: &str| text.parse::<Hotkey>().err().unwrap_or_default();
        assert_eq!(
            error("ctrl+alt+lft"),
            "unknown key `lft` in `ctrl+alt+lft`: use a letter, a digit or left, right, up, down, return, delete, space"
        );
        assert_eq!(
            error("ctrl+alt+K"),
            "unknown key `K` in `ctrl+alt+K`: use a letter, a digit or left, right, up, down, return, delete, space"
        );
        assert_eq!(
            error("control+x"),
            "unknown modifier `control` in `control+x`: use ctrl, alt, opt, cmd, shift"
        );
        assert_eq!(
            error("ctrl+alt"),
            "`ctrl+alt` has no key after its modifiers"
        );
        assert_eq!(error("ctrl+"), "`ctrl+` has no key after its modifiers");
        assert_eq!(error(""), "`` has no key after its modifiers");
        assert_eq!(error("alt+opt+x"), "`alt+opt+x` names a modifier twice");
        assert_eq!(
            error("shift+x"),
            "`shift+x` needs ctrl, alt or cmd, so it doesn't take over typing"
        );
        assert_eq!(
            error("left"),
            "`left` needs ctrl, alt or cmd, so it doesn't take over typing"
        );
        assert!("ctrl + x".parse::<Hotkey>().is_err());
    }

    /// The shell may point stderr at a log file, where a write fails on a full disk. Runs itself
    /// again with stderr on a pipe nobody reads, since macOS has no `/dev/full`.
    #[test]
    fn logging_survives_a_failed_write() -> Result<(), Box<dyn std::error::Error>> {
        if std::env::var_os("LAUNCHER_TEST_LOG_CHILD").is_some() {
            log!("nobody reads this");
            return Ok(());
        }
        let (reader, writer) = std::io::pipe()?;
        drop(reader);
        // `--nocapture`, so a regression to `eprintln!` reaches the pipe rather than libtest.
        let child = std::process::Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "tests::logging_survives_a_failed_write",
                "--nocapture",
            ])
            .env("LAUNCHER_TEST_LOG_CHILD", "1")
            .stderr(writer)
            .output()?;
        assert!(
            child.status.success(),
            "logging failed the test: {}",
            child.status
        );
        // A stale test name would run nothing and pass.
        assert!(String::from_utf8(child.stdout)?.contains("1 passed"));
        Ok(())
    }
}
