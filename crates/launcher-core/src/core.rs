use std::cmp::Reverse;
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Shown on one line, as is the subtitle: `Plugins::query` turns each run of control
    /// characters or line separators into one space, and drops runs at either end.
    pub title: String,
    /// More about the item, such as an app's folder; the group's header already names the
    /// plugin. The shell adds the keys that act on it.
    pub subtitle: String,
    pub icon: Icon,
    pub action: Action,
    /// Shown on request in a scrolling, monospaced view; empty for items without one.
    pub detail: Vec<Section>,
    /// How well the item answers the query, from 0 to 100 on a scale every plugin shares, so the
    /// shell can show the best plugin's results first. 100: the query can only mean this item,
    /// such as a pasted token. About 90: a calculation, or an app whose name starts with the
    /// query. Lower: looser matches.
    pub relevance: u8,
}

impl Item {
    /// A muted row that only says something, such as that work is under way or why there's no
    /// answer.
    pub fn notice(title: &str, subtitle: &str, icon: Icon, relevance: u8) -> Self {
        Self {
            title: title.into(),
            subtitle: subtitle.into(),
            icon,
            action: Action::Nothing,
            detail: Vec::new(),
            relevance,
        }
    }
}

/// What shows beside an item's title. Kinds rather than images keep the core free of platform
/// code: each shell draws them its own way, as macOS does with SF Symbols.
#[derive(Clone, Debug, PartialEq)]
pub enum Icon {
    /// The icon the system shows for the file, such as an app's own.
    File(PathBuf),
    Calculator,
    Token,
    Search,
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
    /// A web address, opened in the browser.
    OpenUrl(String),
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

/// Where settings and state live: `LAUNCHER_CONFIG_DIR` when set, as when measuring or testing
/// the app without the user's own, or else `~/.config/launcher` on every platform.
pub fn config_directory() -> Option<PathBuf> {
    config_directory_from(
        std::env::var_os("LAUNCHER_CONFIG_DIR"),
        std::env::home_dir(),
    )
}

fn config_directory_from(moved: Option<OsString>, home: Option<PathBuf>) -> Option<PathBuf> {
    match moved {
        Some(directory) if !directory.is_empty() => Some(PathBuf::from(directory)),
        _ => home.map(|home| home.join(".config/launcher")),
    }
}

/// The macOS bundle identifier, as `scripts/bundle` sets it, which names the app's cache folder
/// and keychain items.
pub const BUNDLE_IDENTIFIER: &str = "dev.dewet.launcher";

/// Where downloads that can be fetched again live: the platform's cache folder, which backups
/// skip. On macOS it is named for the bundle identifier.
pub fn cache_directory() -> Option<PathBuf> {
    let home = std::env::home_dir()?;
    Some(if cfg!(target_os = "macos") {
        home.join("Library/Caches").join(BUNDLE_IDENTIFIER)
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
    /// Heads the plugin's results.
    fn name(&self) -> &'static str;

    /// Runs on the UI thread for every keystroke, so it must stay well under a frame. The query
    /// is trimmed, and empty once cleared.
    fn query(&mut self, query: &str) -> Vec<Item>;

    /// The shell ran `action` from one of this plugin's items.
    fn ran(&mut self, _action: &Action) {}

    /// The panel hid, so work for the query it showed would go unseen.
    fn hidden(&mut self) {}
}

/// Which plugin produced an item. Only `Plugins::query` makes one, so it always names a plugin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PluginId(usize);

/// One plugin's items for a query, in the plugin's order.
pub struct Group {
    pub plugin: PluginId,
    pub name: &'static str,
    pub items: Vec<Item>,
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

    /// Each plugin's items, the group with the most relevant item first; ties keep the plugins'
    /// order; none for an empty query, though plugins still hear it. Runs on the UI thread for
    /// every keystroke, so every plugin must stay well under a frame.
    pub fn query(&mut self, query: &str) -> Vec<Group> {
        let query = query.trim();
        let mut groups: Vec<Group> = self
            .0
            .iter_mut()
            .enumerate()
            .filter_map(|(index, plugin)| {
                let plugin = plugin.as_mut()?;
                let mut items = plugin.query(query);
                for item in &mut items {
                    make_one_line(&mut item.title);
                    make_one_line(&mut item.subtitle);
                }
                (!items.is_empty()).then(|| Group {
                    plugin: PluginId(index),
                    name: plugin.name(),
                    items,
                })
            })
            .collect();
        // Asked all the same, so plugins can drop work for the query before.
        if query.is_empty() {
            return Vec::new();
        }
        groups.sort_by_key(|group| Reverse(group.items.iter().map(|item| item.relevance).max()));
        groups
    }

    /// The shell hid the panel, or is about to run an item's action; every plugin hears of it.
    pub fn hidden(&mut self) {
        for plugin in self.0.iter_mut().flatten() {
            plugin.hidden();
        }
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

/// The shell draws titles and subtitles on one line, but plugins can pass on text with line
/// breaks or tabs, as a calculator string or a token's claims. Each run of control characters
/// or Unicode line and paragraph separators becomes one space, and runs at either end go.
fn make_one_line(text: &mut String) {
    let breaks =
        |character: char| character.is_control() || matches!(character, '\u{2028}' | '\u{2029}');
    // Almost no text has any, and this runs for every item on every keystroke.
    if text.contains(breaks) {
        *text = text
            .split(breaks)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    type Ran = Arc<Mutex<Vec<Action>>>;

    /// Answers every query with an item per relevance, titled with its name, the query and the
    /// item's place.
    struct Recorder {
        name: &'static str,
        relevances: Vec<u8>,
        ran: Ran,
    }

    impl Recorder {
        fn new(name: &'static str, relevances: &[u8]) -> Self {
            Self {
                name,
                relevances: relevances.to_vec(),
                ran: Ran::default(),
            }
        }
    }

    impl Plugin for Recorder {
        fn name(&self) -> &'static str {
            self.name
        }

        fn query(&mut self, query: &str) -> Vec<Item> {
            self.relevances
                .iter()
                .enumerate()
                .map(|(place, &relevance)| Item {
                    title: format!("{} {query} {place}", self.name),
                    subtitle: String::new(),
                    icon: Icon::Calculator,
                    action: Action::Copy(self.name.into()),
                    detail: Vec::new(),
                    relevance,
                })
                .collect()
        }

        fn ran(&mut self, action: &Action) {
            if let Ok(mut ran) = self.ran.lock() {
                ran.push(action.clone());
            }
        }
    }

    fn recorders() -> (Plugins, Ran, Ran) {
        let first = Recorder::new("first", &[50]);
        let second = Recorder::new("second", &[50]);
        let (first_ran, second_ran) = (first.ran.clone(), second.ran.clone());
        let plugins = Plugins::new(vec![Box::new(first), Box::new(second)]);
        (plugins, first_ran, second_ran)
    }

    /// Each group's name and its items' titles, in the order shown.
    fn shown(plugins: &mut Plugins, query: &str) -> Vec<(&'static str, Vec<String>)> {
        plugins
            .query(query)
            .into_iter()
            .map(|group| {
                let titles = group.items.into_iter().map(|item| item.title).collect();
                (group.name, titles)
            })
            .collect()
    }

    #[test]
    fn the_environment_can_move_the_config_directory() {
        let home = Some(PathBuf::from("/Users/someone"));
        let moved = Some(PathBuf::from("/tmp/measure"));
        assert_eq!(
            config_directory_from(None, home.clone()),
            Some(PathBuf::from("/Users/someone/.config/launcher"))
        );
        assert_eq!(
            config_directory_from(Some("/tmp/measure".into()), home.clone()),
            moved
        );
        assert_eq!(
            config_directory_from(Some("".into()), home),
            Some(PathBuf::from("/Users/someone/.config/launcher")),
            "set but empty, as `LAUNCHER_CONFIG_DIR= Launcher` leaves it"
        );
        assert_eq!(
            config_directory_from(Some("/tmp/measure".into()), None),
            moved
        );
        assert_eq!(config_directory_from(None, None), None);
    }

    #[test]
    fn query_asks_every_plugin_with_the_trimmed_query() {
        let (mut plugins, _, _) = recorders();
        assert_eq!(
            shown(&mut plugins, "  2 + 3 "),
            [
                ("first", vec!["first 2 + 3 0".to_owned()]),
                ("second", vec!["second 2 + 3 0".to_owned()]),
            ]
        );
        assert!(plugins.query("   ").is_empty());
    }

    /// Records what it's asked, answering every query, even an empty one.
    #[derive(Default)]
    struct Listener(Arc<Mutex<Vec<String>>>);

    impl Plugin for Listener {
        fn name(&self) -> &'static str {
            "listener"
        }

        fn query(&mut self, query: &str) -> Vec<Item> {
            if let Ok(mut heard) = self.0.lock() {
                heard.push(format!("query {query:?}"));
            }
            vec![Item::notice(query, "", Icon::Calculator, 50)]
        }

        fn hidden(&mut self) {
            if let Ok(mut heard) = self.0.lock() {
                heard.push("hidden".into());
            }
        }
    }

    #[test]
    fn an_empty_query_reaches_plugins_but_shows_nothing() {
        let listener = Listener::default();
        let heard = listener.0.clone();
        let mut plugins = Plugins::new(vec![Box::new(listener)]);
        assert!(plugins.query("  ").is_empty());
        plugins.hidden();
        assert!(
            heard
                .lock()
                .is_ok_and(|heard| *heard == ["query \"\"", "hidden"])
        );
    }

    #[test]
    fn groups_are_ordered_by_their_best_item() {
        let mut plugins = Plugins::new(vec![
            Box::new(Recorder::new("calculator", &[60])),
            Box::new(Recorder::new("silent", &[])),
            Box::new(Recorder::new("apps", &[40, 90, 20])),
            Box::new(Recorder::new("tied", &[90])),
        ]);
        let order: Vec<(&str, Vec<String>)> = shown(&mut plugins, "q");
        assert_eq!(
            order,
            [
                (
                    "apps",
                    vec!["apps q 0".to_owned(), "apps q 1".into(), "apps q 2".into()]
                ),
                ("tied", vec!["tied q 0".to_owned()]),
                ("calculator", vec!["calculator q 0".to_owned()]),
            ],
            "a plugin's own order stands, ties keep the plugins' order, and empty groups go"
        );
    }

    #[test]
    fn ran_reaches_only_the_producing_plugin() {
        let (mut plugins, first, second) = recorders();
        let groups = plugins.query("anything");
        let Some((group, item)) = groups
            .get(1)
            .and_then(|group| Some((group, group.items.first()?)))
        else {
            panic!("expected a second group with an item, got {}", groups.len());
        };
        plugins.ran(group.plugin, &item.action);
        assert!(first.lock().is_ok_and(|ran| ran.is_empty()));
        assert!(
            second
                .lock()
                .is_ok_and(|ran| *ran == [Action::Copy("second".into())])
        );
    }

    /// Answers with an item whose title, subtitle and copied text are all `text`.
    struct Echo(&'static str);

    impl Plugin for Echo {
        fn name(&self) -> &'static str {
            "echo"
        }

        fn query(&mut self, _query: &str) -> Vec<Item> {
            vec![Item {
                title: self.0.into(),
                subtitle: self.0.into(),
                icon: Icon::Calculator,
                action: Action::Copy(self.0.into()),
                detail: Vec::new(),
                relevance: 50,
            }]
        }
    }

    #[test]
    fn titles_and_subtitles_show_on_one_line() {
        let text = "\na\nb\r\n\tc\u{2028}\u{2029}d  e \t";
        let mut plugins = Plugins::new(vec![Box::new(Echo(text))]);
        let items: Vec<(String, String, Action)> = plugins
            .query("anything")
            .into_iter()
            .flat_map(|group| group.items)
            .map(|item| (item.title, item.subtitle, item.action))
            .collect();
        assert_eq!(
            items,
            [(
                "a b c d  e ".into(),
                "a b c d  e ".into(),
                Action::Copy(text.into())
            )],
            "each run of control characters is one space, spaces stay, and the copy is exact"
        );
    }

    #[test]
    fn emptying_a_place_keeps_the_others_ids() {
        let (mut plugins, _, second) = recorders();
        let groups = plugins.query("anything");
        plugins.set(0, None);
        assert_eq!(
            shown(&mut plugins, "again"),
            [("second", vec!["second again 0".to_owned()])]
        );
        for group in &groups {
            for item in &group.items {
                plugins.ran(group.plugin, &item.action);
            }
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
