use std::cmp::Reverse;
use std::ffi::OsString;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

pub mod markdown;

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
    /// Whether the plugin has sections to show on request, in a scrolling, monospaced view.
    /// They're built by `Plugin::detail` only then, as most items are never opened.
    pub has_detail: bool,
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
            has_detail: false,
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
    Warning,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub title: String,
    pub text: String,
    pub format: Format,
}

/// How a section's text is read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Format {
    /// Shown as is, monospaced, as for JSON.
    Mono,
    /// CommonMark with tables. Text from outside goes through `markdown::escape`.
    Markdown,
}

/// What a shell draws for part of a section. Plugins write sections; `Section::blocks` reads
/// them into blocks, so every shell draws the same thing.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// Proportional text that wraps.
    Paragraph(Vec<Span>),
    /// Monospaced, its lines kept.
    Code(String),
    /// Every item of a list and the lists inside it, in order.
    List(Vec<ListItem>),
    /// Rows of cells, under a header row of cells.
    Table {
        header: Vec<Vec<Span>>,
        rows: Vec<Vec<Vec<Span>>>,
    },
    /// A picture in a file, read only while the detail shows. A plugin downloads one in its
    /// background work, into `cache_directory`.
    Image {
        path: PathBuf,
        /// Copied in its place, and shown if it can't be read.
        description: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    /// 0 for the outermost list.
    pub depth: u8,
    /// Such as `•` or `3.`; empty for text after a list inside the item.
    pub marker: String,
    pub spans: Vec<Span>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub text: String,
    pub style: Style,
    /// A web or email address the text opens.
    pub link: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    /// Monospaced, as for a name in code.
    pub code: bool,
}

impl Section {
    pub fn mono(title: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            text: text.into(),
            format: Format::Mono,
        }
    }

    pub fn markdown(title: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            format: Format::Markdown,
            ..Self::mono(title, text)
        }
    }

    /// Read now, as most details are never opened.
    pub fn blocks(&self) -> Vec<Block> {
        match self.format {
            Format::Mono => vec![Block::Code(self.text.clone())],
            Format::Markdown => markdown::blocks(&self.text),
        }
    }

    /// What copying the whole section gives: its text without the Markdown.
    pub fn plain_text(&self) -> String {
        let blocks: Vec<String> = self.blocks().iter().map(Block::plain_text).collect();
        blocks.join("\n")
    }
}

impl Block {
    /// Lists as their markers show them, and tables tab-separated, as a spreadsheet pastes them.
    fn plain_text(&self) -> String {
        fn text(spans: &[Span]) -> String {
            spans.iter().map(|span| span.text.as_str()).collect()
        }
        let lines: Vec<String> = match self {
            Self::Paragraph(spans) => return text(spans),
            Self::Code(code) => return code.clone(),
            Self::Image { description, .. } => return description.clone(),
            Self::List(items) => items
                .iter()
                .map(|item| {
                    let indent = "  ".repeat(item.depth.into());
                    match item.marker.as_str() {
                        "" => format!("{indent}{}", text(&item.spans)),
                        marker => format!("{indent}{marker} {}", text(&item.spans)),
                    }
                })
                .collect(),
            Self::Table { header, rows } => std::iter::once(header)
                .chain(rows)
                .map(|row| {
                    let cells: Vec<String> = row.iter().map(|cell| text(cell)).collect();
                    cells.join("\t")
                })
                .collect(),
        };
        lines.join("\n")
    }
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

/// For a test that checks what's logged: `None` in the copy of the test `name` (its full path,
/// such as `tests::logs_once`) this starts, which should then log; otherwise that copy's output,
/// once it has passed, with its stderr sent to `stderr`. `--nocapture`, so a regression to
/// `eprintln!` reaches `stderr` rather than libtest.
#[cfg(any(test, feature = "test-support"))]
pub fn run_test_again(
    name: &str,
    stderr: std::process::Stdio,
) -> Result<Option<std::process::Output>, Box<dyn std::error::Error>> {
    const AGAIN: &str = "LAUNCHER_TEST_AGAIN";
    if std::env::var_os(AGAIN).is_some() {
        return Ok(None);
    }
    let again = std::process::Command::new(std::env::current_exe()?)
        .args(["--exact", name, "--nocapture"])
        .env(AGAIN, "1")
        .stderr(stderr)
        .output()?;
    if !again.status.success() {
        return Err(format!(
            "{name} failed when run again ({}):\n{}{}",
            again.status,
            String::from_utf8_lossy(&again.stdout),
            String::from_utf8_lossy(&again.stderr)
        )
        .into());
    }
    // A stale test name would run nothing and pass.
    if !String::from_utf8_lossy(&again.stdout).contains("1 passed") {
        return Err(format!("no test {name} to run again").into());
    }
    Ok(Some(again))
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

    /// The sections shown on request for `item`, which this plugin returned for `query`.
    fn detail(&mut self, _query: &str, _item: &Item) -> Vec<Section> {
        Vec::new()
    }

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

/// The title of the notice a crashed plugin shows in place of its results.
pub const CRASHED: &str = "Crashed, so it's off";

/// The plugins a shell runs, asked in order. A place can be empty, for a plugin that's off, so
/// turning one on or off leaves the others' ids, and the items they made, pointing at them.
pub struct Plugins(Vec<Slot>);

enum Slot {
    Off,
    On(Box<dyn Plugin + Send>),
    /// Turned off by a panic, until started again; it says so in place of its results.
    Crashed(&'static str),
}

impl Plugins {
    pub fn new(plugins: Vec<Box<dyn Plugin + Send>>) -> Self {
        Self(plugins.into_iter().map(Slot::On).collect())
    }

    /// `count` empty places, for `start` to fill.
    pub fn empty(count: usize) -> Self {
        Self(std::iter::repeat_with(|| Slot::Off).take(count).collect())
    }

    /// Puts the plugin `start` makes, named `name`, in place `place`, dropping the plugin there.
    /// If `start` panics, the place shows that the plugin crashed.
    pub fn start(
        &mut self,
        place: usize,
        name: &'static str,
        start: impl FnOnce() -> Box<dyn Plugin + Send>,
    ) {
        let started = catch_plugin_panic(name, start).map_or(Slot::Crashed(name), Slot::On);
        self.put(place, started);
    }

    /// Empties place `place`, dropping the plugin there.
    pub fn turn_off(&mut self, place: usize) {
        self.put(place, Slot::Off);
    }

    /// Whether the plugin in place `place` panicked, and hasn't been started again since.
    pub fn crashed(&self, place: usize) -> bool {
        matches!(self.0.get(place), Some(Slot::Crashed(_)))
    }

    fn put(&mut self, place: usize, slot: Slot) {
        match self.0.get_mut(place) {
            Some(old) => drop_plugin(std::mem::replace(old, slot)),
            None => log!("no place {place} for a plugin"),
        }
    }

    /// Each plugin's items, the group with the most relevant item first; ties keep the plugins'
    /// order; none for an empty query, though plugins still hear it. Runs on the UI thread for
    /// every keystroke, so every plugin must stay well under a frame.
    pub fn query(&mut self, query: &str) -> Vec<Group> {
        let query = query.trim();
        let mut groups: Vec<Group> = (0..self.0.len())
            .filter_map(|place| {
                let answer = self.ask(place, |plugin| (plugin.name(), plugin.query(query)));
                let (name, mut items) = match (answer, self.0.get(place)) {
                    (Some(answer), _) => answer,
                    (None, Some(&Slot::Crashed(name))) => (name, vec![crashed_notice()]),
                    (None, _) => return None,
                };
                for item in &mut items {
                    make_one_line(&mut item.title);
                    make_one_line(&mut item.subtitle);
                }
                (!items.is_empty()).then_some(Group {
                    plugin: PluginId(place),
                    name,
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
        for place in 0..self.0.len() {
            self.ask(place, |plugin| plugin.hidden());
        }
    }

    /// The sections shown on request for `item`, which `plugin` returned for `query`; none if
    /// the plugin is gone or panics.
    pub fn detail(&mut self, plugin: PluginId, query: &str, item: &Item) -> Vec<Section> {
        self.ask(plugin.0, |found| found.detail(query.trim(), item))
            .unwrap_or_default()
    }

    /// The shell ran `action` from an item `plugin` produced; only that plugin hears of it.
    pub fn ran(&mut self, plugin: PluginId, action: &Action) {
        if let Some(Slot::On(_)) = self.0.get(plugin.0) {
            self.ask(plugin.0, |found| found.ran(action));
        } else {
            // Not the action: a copy's text can be a token's claims, and the log is kept on disk.
            log!("no plugin {} to tell it ran an action", plugin.0);
        }
    }

    /// `call` with the plugin in `place`; `None` if there's none or it panicked. One that panics
    /// is dropped rather than asked again, as its state may be broken.
    fn ask<T>(
        &mut self,
        place: usize,
        call: impl FnOnce(&mut (dyn Plugin + Send)) -> T,
    ) -> Option<T> {
        let Some(Slot::On(plugin)) = self.0.get_mut(place) else {
            return None;
        };
        let name = plugin.name();
        let answer = catch_plugin_panic(name, || call(plugin.as_mut()));
        if answer.is_none() {
            self.put(place, Slot::Crashed(name));
        }
        answer
    }
}

/// Last, as it answers no query, and muted, as there's nothing to run.
fn crashed_notice() -> Item {
    Item::notice(
        CRASHED,
        "Reload Settings to start it again",
        Icon::Warning,
        0,
    )
}

/// A plugin's `Drop` is its code too, so a panic there is caught as well.
fn drop_plugin(slot: Slot) {
    if let Slot::On(plugin) = slot {
        catch_plugin_panic(plugin.name(), || drop(plugin));
    }
}

/// `call`, which starts or asks the plugin `name`; `None` if it panicked, logging that the
/// plugin is off. So a plugin's bug costs only its own results: the shell would otherwise go
/// down with it.
fn catch_plugin_panic<T>(name: &str, call: impl FnOnce() -> T) -> Option<T> {
    let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)).ok();
    if answer.is_none() {
        log!("the {name} plugin panicked, so it's off");
    }
    answer
}

/// Which shown item is picked: the top one for a new query. Once the user moves the selection
/// (↑, ↓, hover) or opens an item's detail, a refresh for the same query keeps it on that item,
/// as results arriving in the background come and go around it.
#[derive(Default)]
pub struct Selection {
    query: String,
    /// A hash of each shown item's plugin, title and subtitle, to find the picked one among the
    /// next results without keeping their text. Two of a few dozen rows sharing one is unlikely
    /// enough to ignore.
    shown: Vec<u64>,
    /// The item the user moved to; the top one while `None`.
    moved_to: Option<usize>,
}

impl Selection {
    /// Takes `groups`, shown for `query`, and returns the index of the picked item among their
    /// items, counted across groups.
    pub fn show(&mut self, query: &str, groups: &[Group]) -> usize {
        let kept = self
            .moved_to
            .filter(|_| query == self.query)
            .and_then(|index| self.shown.get(index).copied());
        // Refilled rather than rebuilt, as this runs on every keystroke.
        self.shown.clear();
        self.shown.extend(groups.iter().flat_map(|group| {
            group.items.iter().map(|item| {
                let mut hasher = DefaultHasher::new();
                (group.plugin.0, &item.title, &item.subtitle).hash(&mut hasher);
                hasher.finish()
            })
        }));
        self.moved_to = kept.and_then(|kept| self.shown.iter().position(|&item| item == kept));
        query.clone_into(&mut self.query);
        self.picked()
    }

    /// The user picked the `index`th item, by pointing at it or opening its detail. An index
    /// past the items shown leaves the selection where it is.
    pub fn pick(&mut self, index: usize) -> usize {
        if index < self.shown.len() {
            self.moved_to = Some(index);
        }
        self.picked()
    }

    /// ↓: the next item, wrapping from the last to the first.
    pub fn down(&mut self) -> usize {
        let count = self.shown.len();
        if count == 0 {
            return 0;
        }
        self.pick((self.picked() + 1) % count)
    }

    /// ↑: the previous item, wrapping from the first to the last.
    pub fn up(&mut self) -> usize {
        let count = self.shown.len();
        if count == 0 {
            return 0;
        }
        self.pick((self.picked() + count - 1) % count)
    }

    fn picked(&self) -> usize {
        self.moved_to.unwrap_or(0)
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
                    has_detail: false,
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

    /// Panics whatever it's asked.
    struct Panicking;

    impl Plugin for Panicking {
        fn name(&self) -> &'static str {
            "panicking"
        }

        fn query(&mut self, _query: &str) -> Vec<Item> {
            panic!("a bug in query");
        }

        fn ran(&mut self, _action: &Action) {
            panic!("a bug in ran");
        }

        fn hidden(&mut self) {
            panic!("a bug in hidden");
        }
    }

    /// Runs itself again with stderr on a pipe, to count what it logs.
    #[test]
    fn a_plugin_that_panics_is_turned_off() -> Result<(), Box<dyn std::error::Error>> {
        let Some(again) = run_test_again(
            "tests::a_plugin_that_panics_is_turned_off",
            std::process::Stdio::piped(),
        )?
        else {
            let calm = Recorder::new("calm", &[50]);
            let mut plugins = Plugins::new(vec![Box::new(Panicking), Box::new(calm)]);
            for query in ["a", "ab"] {
                assert_eq!(
                    shown(&mut plugins, query),
                    [
                        ("calm", vec![format!("calm {query} 0")]),
                        ("panicking", vec![CRASHED.into()])
                    ]
                );
            }
            plugins.hidden();
            assert!(plugins.crashed(0));
            assert!(!plugins.crashed(1));
            return Ok(());
        };
        let logged = String::from_utf8(again.stderr)?;
        assert_eq!(
            logged.matches("the panicking plugin").count(),
            1,
            "{logged}"
        );
        Ok(())
    }

    #[test]
    fn a_plugin_that_panics_starting_says_it_crashed() {
        let mut plugins = Plugins::empty(1);
        plugins.start(0, "panicking", || panic!("a bug in start"));
        assert!(plugins.crashed(0));
        let crashed = vec![CRASHED.to_owned()];
        assert_eq!(shown(&mut plugins, "a"), [("panicking", crashed)]);
        assert_eq!(shown(&mut plugins, ""), []);
    }

    #[test]
    fn a_crashed_plugin_started_again_answers() {
        let mut plugins = Plugins::new(vec![Box::new(Panicking)]);
        plugins.hidden();
        plugins.start(0, "calm", || Box::new(Recorder::new("calm", &[50])));
        assert!(!plugins.crashed(0));
        assert_eq!(
            shown(&mut plugins, "a"),
            [("calm", vec!["calm a 0".into()])]
        );
        plugins.hidden();
        plugins.turn_off(0);
        assert!(!plugins.crashed(0));
        assert_eq!(shown(&mut plugins, "a"), []);
    }

    #[test]
    fn hiding_or_running_an_action_survives_a_plugin_that_panics() {
        let (mut plugins, _, second_ran) = recorders();
        plugins.start(0, "panicking", || Box::new(Panicking));
        plugins.hidden();
        assert!(plugins.crashed(0));
        plugins.start(0, "panicking", || Box::new(Panicking));
        plugins.ran(PluginId(0), &Action::Nothing);
        plugins.ran(PluginId(1), &Action::Nothing);
        assert!(second_ran.lock().is_ok_and(|ran| *ran == [Action::Nothing]));
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
                has_detail: false,
                relevance: 50,
            }]
        }
    }

    /// One group per plugin, each item titled with its own name. A plugin keeps its id
    /// whichever order the groups come in, as in `Plugins`.
    fn groups(plugins: &[(&'static str, &[&str])]) -> Vec<Group> {
        plugins
            .iter()
            .map(|(name, titles)| Group {
                plugin: PluginId(
                    ["Calculator", "Apps"]
                        .iter()
                        .position(|place| place == name)
                        .unwrap_or(2),
                ),
                name,
                items: titles
                    .iter()
                    .map(|title| Item::notice(title, "", Icon::Calculator, 50))
                    .collect(),
            })
            .collect()
    }

    #[test]
    fn a_new_query_picks_the_top_result() {
        let mut selection = Selection::default();
        assert_eq!(
            selection.show("pi", &groups(&[("Calculator", &["3.14"])])),
            0
        );
        selection.down();
        let apps = groups(&[("Apps", &["Pixelmator", "Pi-hole"])]);
        assert_eq!(selection.show("pix", &apps), 0);
    }

    /// As when the calculator answered `pi` before the apps loaded, and apps rank above it.
    #[test]
    fn a_refresh_picks_the_new_top_result_until_the_selection_moves() {
        let mut selection = Selection::default();
        selection.show("pi", &groups(&[("Calculator", &["3.14"])]));
        let loaded = groups(&[
            ("Apps", &["Raspberry Pi Imager"]),
            ("Calculator", &["3.14"]),
        ]);
        assert_eq!(selection.show("pi", &loaded), 0);
    }

    #[test]
    fn a_moved_selection_follows_its_item_across_refreshes() {
        let mut selection = Selection::default();
        selection.show("ma", &groups(&[("Apps", &["Mail", "Maps"])]));
        assert_eq!(selection.down(), 1);
        let more = groups(&[("Calculator", &["ma"]), ("Apps", &["Mail", "Maps"])]);
        assert_eq!(selection.show("ma", &more), 2, "Maps, below the new row");
        assert_eq!(
            selection.show("ma", &more),
            2,
            "and still after another refresh"
        );
        let gone = groups(&[("Apps", &["Mail"])]);
        assert_eq!(
            selection.show("ma", &gone),
            0,
            "the top result once Maps goes"
        );
        let back = groups(&[("Apps", &["Mail", "Maps"])]);
        assert_eq!(selection.show("ma", &back), 0, "which isn't a move");
    }

    /// Typing that gives the same query again, as pasting it over itself, is no new query.
    #[test]
    fn the_same_query_again_keeps_a_moved_selection() {
        let mut selection = Selection::default();
        let apps = groups(&[("Apps", &["Mail", "Maps"])]);
        selection.show("ma", &apps);
        selection.down();
        assert_eq!(selection.show("ma", &apps), 1);
    }

    #[test]
    fn picking_an_item_counts_as_moving() {
        let mut selection = Selection::default();
        let apps = groups(&[("Apps", &["Mail", "Maps"])]);
        selection.show("ma", &apps);
        assert_eq!(selection.pick(1), 1);
        assert_eq!(selection.pick(5), 1, "past the items, nothing changes");
        assert_eq!(selection.show("ma", &apps), 1);
        assert_eq!(
            selection.show("map", &apps),
            0,
            "a new query starts at the top"
        );
    }

    #[test]
    fn up_and_down_wrap_across_groups() {
        let mut selection = Selection::default();
        selection.show(
            "e",
            &groups(&[("Calculator", &["e"]), ("Apps", &["Mail", "Notes"])]),
        );
        assert_eq!(
            selection.up(),
            2,
            "from the first to the last, past the headers"
        );
        assert_eq!(selection.down(), 0);
        assert_eq!(selection.down(), 1, "into the next group");
        selection.show("", &[]);
        assert_eq!(
            (selection.up(), selection.down()),
            (0, 0),
            "nothing to move through"
        );
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
        plugins.turn_off(0);
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
        let (reader, writer) = std::io::pipe()?;
        drop(reader);
        if run_test_again("tests::logging_survives_a_failed_write", writer.into())?.is_none() {
            log!("nobody reads this");
        }
        Ok(())
    }
}
