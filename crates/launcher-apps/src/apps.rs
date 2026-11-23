use std::cmp::Reverse;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use launcher_core::{Action, Item, Plugin, config_directory};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

const MAX_RESULTS: usize = 8;
const MAX_REMEMBERED: usize = 500;
/// Installs and updates touch the folders many times in a burst; rescan once it settles.
const SETTLE: Duration = Duration::from_millis(500);

const MATCH: i32 = 16;
const WORD_START: i32 = 12;
/// Below `WORD_START`, so "ss" prefers System Settings to Chess.
const CONSECUTIVE: i32 = 4;
const GAP_START: i32 = 3;
const GAP_EXTEND: i32 = 1;
/// Frecency reorders comparable matches but never lifts a scattered match over a clean one.
const MAX_FRECENCY_BONUS: i32 = 24;

const HOUR: u64 = 60 * 60;
const DAY: u64 = 24 * HOUR;
const WEEK: u64 = 7 * DAY;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct App {
    name: String,
    path: PathBuf,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Usage {
    count: u32,
    last_opened: u64,
}

impl Usage {
    fn weight(self, now: u64) -> f64 {
        let recency = match now.saturating_sub(self.last_opened) {
            age if age < HOUR => 4.0,
            age if age < DAY => 2.0,
            age if age < WEEK => 0.5,
            _ => 0.25,
        };
        f64::from(self.count) * recency
    }
}

/// Searches application bundles by name, ranked by match quality and frecency.
pub struct Apps {
    index: Arc<Mutex<Vec<App>>>,
    usage: HashMap<PathBuf, Usage>,
    // Writes run on one thread, in order, so a later save can't land before an earlier one.
    saver: Option<Sender<String>>,
    // Watching stops when this drops, which also ends the rescan thread.
    _watcher: Option<RecommendedWatcher>,
}

impl Apps {
    /// Indexes the app folders in the background and keeps the index current, with frecency
    /// kept in the config directory.
    pub fn start() -> Self {
        let frecency_path = config_directory().map(|directory| directory.join("frecency"));
        if frecency_path.is_none() {
            eprintln!("no home folder, so apps are ranked without frecency");
        }
        Self::watching(app_folders(), frecency_path)
    }

    /// Without a `frecency_path`, apps are ranked by match alone.
    fn watching(roots: Vec<PathBuf>, frecency_path: Option<PathBuf>) -> Self {
        let index = Arc::new(Mutex::new(Vec::new()));
        let (sender, receiver) = mpsc::channel();
        let watcher = watch(&roots, sender);
        thread::spawn({
            let index = index.clone();
            move || keep_index(&index, &roots, &receiver)
        });
        let usage = frecency_path.as_deref().map(load_usage).unwrap_or_default();
        let saver = frecency_path.map(|path| {
            let (sender, receiver) = mpsc::channel::<String>();
            thread::spawn(move || {
                for contents in receiver {
                    if let Err(error) = save(&path, &contents) {
                        eprintln!("can't save {}: {error}", path.display());
                    }
                }
            });
            sender
        });
        Self {
            index,
            usage,
            saver,
            _watcher: watcher,
        }
    }
}

impl Plugin for Apps {
    fn query(&mut self, query: &str) -> Vec<Item> {
        let Ok(index) = self.index.lock() else {
            eprintln!("the app index thread panicked; skipping apps");
            return Vec::new();
        };
        rank(&index, &self.usage, query, now())
            .into_iter()
            .take(MAX_RESULTS)
            .map(|app| Item {
                title: app.name.clone(),
                subtitle: app
                    .path
                    .parent()
                    .map(|folder| folder.display().to_string())
                    .unwrap_or_default(),
                action: Action::Open(app.path.clone()),
            })
            .collect()
    }

    fn opened(&mut self, path: &Path) {
        let Some(saver) = &self.saver else {
            return;
        };
        let now = now();
        let usage = self.usage.entry(path.to_path_buf()).or_default();
        usage.count = usage.count.saturating_add(1);
        usage.last_opened = now;
        if self.usage.len() > MAX_REMEMBERED
            && let Some(weakest) = self
                .usage
                .iter()
                .min_by(|left, right| left.1.weight(now).total_cmp(&right.1.weight(now)))
                .map(|(path, _)| path.clone())
        {
            self.usage.remove(&weakest);
        }
        if saver.send(serialize_usage(&self.usage)).is_err() {
            eprintln!("the frecency writer stopped; not saving");
        }
    }
}

/// The folders macOS installs apps into. Linux will index `.desktop` files instead.
fn app_folders() -> Vec<PathBuf> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
    ];
    if let Some(home) = std::env::home_dir() {
        roots.push(home.join("Applications"));
    }
    roots
}

fn keep_index(index: &Mutex<Vec<App>>, roots: &[PathBuf], changes: &Receiver<()>) {
    let replace = |apps| match index.lock() {
        Ok(mut index) => *index = apps,
        Err(_) => eprintln!("the app index is poisoned; not refreshing it"),
    };
    replace(scan(roots));
    while changes.recv().is_ok() {
        while changes.recv_timeout(SETTLE).is_ok() {}
        replace(scan(roots));
    }
}

fn watch(roots: &[PathBuf], changes: Sender<()>) -> Option<RecommendedWatcher> {
    let handler = move |result: notify::Result<notify::Event>| match result {
        Ok(event) if event.paths.iter().any(|path| !inside_bundle(path)) => {
            if changes.send(()).is_err() {
                eprintln!("the app index thread stopped; ignoring folder changes");
            }
        }
        Ok(_) => {}
        Err(error) => eprintln!("app folder watcher: {error}"),
    };
    let mut watcher = match notify::recommended_watcher(handler) {
        Ok(watcher) => watcher,
        Err(error) => {
            eprintln!("can't watch app folders, so new apps need a restart: {error}");
            return None;
        }
    };
    for root in roots.iter().filter(|root| root.is_dir()) {
        if let Err(error) = watcher.watch(root, RecursiveMode::Recursive) {
            eprintln!("can't watch {}: {error}", root.display());
        }
    }
    Some(watcher)
}

/// Apps write inside their own bundles (updates, caches); only bundles appearing or going
/// away change the index.
fn inside_bundle(path: &Path) -> bool {
    path.ancestors().skip(1).any(is_bundle)
}

fn is_bundle(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "app")
}

fn scan(roots: &[PathBuf]) -> Vec<App> {
    let mut apps = Vec::new();
    for root in roots {
        // One level of plain folders, for vendor folders such as `/Applications/Adobe …/`.
        collect_apps(root, 1, &mut apps);
    }
    apps.sort();
    apps.dedup();
    apps
}

fn collect_apps(folder: &Path, depth: usize, apps: &mut Vec<App>) {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(error) => {
            eprintln!("can't list {}: {error}", folder.display());
            return;
        }
    };
    for entry in entries {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(error) => {
                eprintln!("can't list {}: {error}", folder.display());
                continue;
            }
        };
        if is_bundle(&path)
            && let Some(name) = path.file_stem()
        {
            apps.push(App {
                name: name.to_string_lossy().into_owned(),
                path,
            });
        } else if depth > 0 && path.is_dir() {
            collect_apps(&path, depth - 1, apps);
        }
    }
}

/// Matching apps, best first. Ties keep the index's name order.
fn rank<'a>(
    apps: &'a [App],
    usage: &HashMap<PathBuf, Usage>,
    query: &str,
    now: u64,
) -> Vec<&'a App> {
    let query = fold_query(query);
    if query.is_empty() {
        return Vec::new();
    }
    let mut ranked: Vec<(i32, &App)> = apps
        .iter()
        .filter_map(|app| {
            let score = score(&app.name, &query)?;
            let bonus = usage
                .get(&app.path)
                .map_or(0, |usage| frecency_bonus(usage.weight(now)));
            Some((score + bonus, app))
        })
        .collect();
    ranked.sort_by_key(|(score, _)| Reverse(*score));
    ranked.into_iter().map(|(_, app)| app).collect()
}

fn frecency_bonus(weight: f64) -> i32 {
    // Saturating float-to-int cast; the weight is never negative.
    ((weight.ln_1p() * 8.0) as i32).min(MAX_FRECENCY_BONUS)
}

/// The query's letters, case-folded and without spaces, so "studio code" matches as well.
fn fold_query(query: &str) -> Vec<char> {
    query
        .chars()
        .filter(|character| !character.is_whitespace())
        .map(fold_case)
        .collect()
}

/// How well the folded `query`'s letters match `name` in order, or `None` if they don't.
/// Matches at word starts, which acronyms hit, and runs of consecutive letters score higher;
/// gaps and longer names score lower.
fn score(name: &str, query: &[char]) -> Option<i32> {
    let (first, rest) = query.split_first()?;
    let name: Vec<char> = name.chars().collect();
    let folded: Vec<char> = name.iter().copied().map(fold_case).collect();
    let previous_characters = std::iter::once(None).chain(name.iter().copied().map(Some));
    let bonuses: Vec<i32> = previous_characters
        .zip(name.iter().copied())
        .map(|(previous, current)| {
            if is_word_start(previous, current) {
                WORD_START
            } else {
                0
            }
        })
        .collect();

    // `best[index]`: the best score with the current query letter matched at `name[index]`.
    let mut best: Vec<Option<i32>> = folded
        .iter()
        .zip(&bonuses)
        .map(|(character, bonus)| (character == first).then_some(MATCH + bonus))
        .collect();
    for letter in rest {
        let mut next = vec![None; name.len()];
        // The best earlier match with at least one letter skipped before `index`.
        let mut across_gap: Option<i32> = None;
        for index in 0..name.len() {
            if index >= 2 {
                across_gap = across_gap
                    .map(|score| score - GAP_EXTEND)
                    .max(best[index - 2].map(|score| score - GAP_START));
            }
            if folded[index] != *letter {
                continue;
            }
            let consecutive = index
                .checked_sub(1)
                .and_then(|previous| best[previous])
                .map(|score| score + CONSECUTIVE);
            next[index] = consecutive
                .max(across_gap)
                .map(|score| score + MATCH + bonuses[index]);
        }
        best = next;
    }
    // Only a tie-breaker between similar matches, so it stays small next to a word start.
    let length_penalty = i32::try_from(name.len() / 2).unwrap_or(i32::MAX);
    best.into_iter()
        .flatten()
        .max()
        .map(|score| score.saturating_sub(length_penalty))
}

fn fold_case(character: char) -> char {
    character.to_lowercase().next().unwrap_or(character)
}

fn is_word_start(previous: Option<char>, current: char) -> bool {
    previous.is_none_or(|previous| {
        !previous.is_alphanumeric() || (previous.is_lowercase() && current.is_uppercase())
    })
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn load_usage(path: &Path) -> HashMap<PathBuf, Usage> {
    match fs::read_to_string(path) {
        Ok(contents) => parse_usage(&contents),
        Err(error) if error.kind() == io::ErrorKind::NotFound => HashMap::new(),
        Err(error) => {
            eprintln!("can't read {}: {error}", path.display());
            HashMap::new()
        }
    }
}

/// One `count\tlast_opened\tpath` line per app.
fn parse_usage(contents: &str) -> HashMap<PathBuf, Usage> {
    contents
        .lines()
        .filter_map(|line| {
            let parsed = parse_usage_line(line);
            if parsed.is_none() {
                eprintln!("skipping unreadable frecency line: {line:?}");
            }
            parsed
        })
        .collect()
}

fn parse_usage_line(line: &str) -> Option<(PathBuf, Usage)> {
    let mut fields = line.splitn(3, '\t');
    let count = fields.next()?.parse().ok()?;
    let last_opened = fields.next()?.parse().ok()?;
    let path = PathBuf::from(fields.next()?);
    Some((path, Usage { count, last_opened }))
}

fn serialize_usage(usage: &HashMap<PathBuf, Usage>) -> String {
    usage
        .iter()
        .map(|(path, usage)| {
            format!(
                "{}\t{}\t{}\n",
                usage.count,
                usage.last_opened,
                path.display()
            )
        })
        .collect()
}

/// Writes to a temporary file first, so a crash mid-write can't truncate the history.
fn save(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, contents)?;
    fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apps(names: &[&str]) -> Vec<App> {
        names
            .iter()
            .map(|name| App {
                name: (*name).to_owned(),
                path: PathBuf::from(format!("/Applications/{name}.app")),
            })
            .collect()
    }

    fn ranked_names(apps: &[App], usage: &HashMap<PathBuf, Usage>, query: &str) -> Vec<String> {
        rank(apps, usage, query, 1_000_000)
            .into_iter()
            .map(|app| app.name.clone())
            .collect()
    }

    fn matches(name: &str, query: &str) -> Option<i32> {
        score(name, &fold_query(query))
    }

    fn eventually(mut condition: impl FnMut() -> bool) -> bool {
        for _ in 0..100 {
            if condition() {
                return true;
            }
            thread::sleep(Duration::from_millis(50));
        }
        false
    }

    const INSTALLED: &[&str] = &[
        "App Store",
        "Calendar",
        "Font Book",
        "Mail",
        "Maps",
        "Notes",
        "Safari",
        "Safe Exam Browser",
        "Screen Sharing",
        "Slack",
        "System Settings",
        "TextEdit",
        "Visual Studio Code",
        "Xcode",
    ];

    #[test]
    fn prefix_ranks_first() {
        let installed = apps(INSTALLED);
        let names = ranked_names(&installed, &HashMap::new(), "saf");
        assert_eq!(names, ["Safari", "Safe Exam Browser"]);
    }

    #[test]
    fn acronym_matches_word_starts() {
        let installed = apps(INSTALLED);
        let names = ranked_names(&installed, &HashMap::new(), "vsc");
        assert_eq!(
            names.first().map(String::as_str),
            Some("Visual Studio Code")
        );
        let names = ranked_names(&installed, &HashMap::new(), "ss");
        assert_eq!(
            &names[..2],
            ["Screen Sharing", "System Settings"],
            "two word starts beat a word start and a mid-word s"
        );
    }

    #[test]
    fn letters_must_be_in_order() {
        assert_eq!(matches("Safari", "fas"), None);
        assert_eq!(matches("Safari", ""), None);
        assert!(matches("Safari", "SAF").is_some());
        assert!(matches("Visual Studio Code", "studio code").is_some());
    }

    #[test]
    fn word_start_beats_mid_word() {
        assert!(matches("Text Edit", "e") > matches("Tbe Edit", "b"));
        assert!(
            matches("TextEdit", "e") > matches("Tbe Edit", "b"),
            "camel-case hump"
        );
        assert!(
            matches("System Settings", "ss") > matches("Chess", "ss"),
            "word starts beat a consecutive run mid-word"
        );
    }

    #[test]
    fn non_ascii_names_dont_panic() {
        assert!(matches("Ünïcødé Äpp", "üä").is_some());
        assert!(matches("İstanbul", "i").is_some());
    }

    #[test]
    fn frequently_opened_app_rises() {
        let installed = apps(INSTALLED);
        let mut usage = HashMap::new();
        assert_eq!(ranked_names(&installed, &usage, "ma"), ["Mail", "Maps"]);
        usage.insert(
            PathBuf::from("/Applications/Maps.app"),
            Usage {
                count: 3,
                last_opened: 1_000_000,
            },
        );
        assert_eq!(ranked_names(&installed, &usage, "ma"), ["Maps", "Mail"]);
    }

    #[test]
    fn frecency_doesnt_lift_a_scattered_match() {
        let installed = apps(&["Notes", "Annotate Hosts"]);
        let usage = HashMap::from([(
            PathBuf::from("/Applications/Annotate Hosts.app"),
            Usage {
                count: 10_000,
                last_opened: 1_000_000,
            },
        )]);
        assert_eq!(
            ranked_names(&installed, &usage, "notes"),
            ["Notes", "Annotate Hosts"]
        );
    }

    #[test]
    fn usage_round_trips_and_skips_corrupt_lines() -> io::Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("nested/frecency");
        let usage = HashMap::from([
            (
                PathBuf::from("/Applications/Safari.app"),
                Usage {
                    count: 3,
                    last_opened: 42,
                },
            ),
            (
                PathBuf::from("/Applications/Tab\tName.app"),
                Usage {
                    count: 1,
                    last_opened: 7,
                },
            ),
        ]);
        save(&path, &serialize_usage(&usage))?;
        assert_eq!(load_usage(&path), usage);
        assert_eq!(
            parse_usage("garbage\n2\t5\t/Applications/Mail.app\nx\t1\t/a.app\n"),
            HashMap::from([(
                PathBuf::from("/Applications/Mail.app"),
                Usage {
                    count: 2,
                    last_opened: 5
                }
            )])
        );
        assert!(load_usage(&folder.path().join("missing")).is_empty());
        Ok(())
    }

    #[test]
    fn opened_records_usage() -> io::Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("frecency");
        let mut plugin = Apps::watching(Vec::new(), Some(path.clone()));
        let safari = Path::new("/Applications/Safari.app");
        plugin.opened(safari);
        plugin.opened(safari);
        assert!(eventually(|| load_usage(&path)
            .get(safari)
            .is_some_and(|usage| usage.count == 2)));
        Ok(())
    }

    #[test]
    fn scan_finds_bundles_one_folder_deep() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        for folder in [
            "Safari.app/Contents/Helper.app",
            "Vendor/Tool.app",
            "Vendor/Deeper/Hidden.app",
            "Not An App",
        ] {
            fs::create_dir_all(root.path().join(folder))?;
        }
        let names = |roots: &[PathBuf]| -> Vec<String> {
            scan(roots).into_iter().map(|app| app.name).collect()
        };
        let missing = root.path().join("missing");
        assert_eq!(
            names(&[root.path().to_path_buf(), missing]),
            ["Safari", "Tool"]
        );
        // `/System/Applications/Utilities` is a root and inside one.
        let vendor = root.path().join("Vendor");
        assert_eq!(
            names(&[root.path().to_path_buf(), vendor]),
            ["Hidden", "Safari", "Tool"]
        );
        Ok(())
    }

    #[test]
    fn ignores_changes_inside_bundles() {
        assert!(!inside_bundle(Path::new("/Applications/Safari.app")));
        assert!(!inside_bundle(Path::new("/Applications/Vendor")));
        assert!(inside_bundle(Path::new(
            "/Applications/Safari.app/Contents/Info.plist"
        )));
    }

    #[test]
    fn new_apps_appear_without_restart() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("Safari.app"))?;
        let mut plugin = Apps::watching(vec![root.path().to_path_buf()], None);
        let mut titles = |query: &str| -> Vec<String> {
            plugin
                .query(query)
                .into_iter()
                .map(|item| item.title)
                .collect()
        };
        assert!(eventually(|| titles("saf") == ["Safari"]));
        fs::create_dir(root.path().join("Zed.app"))?;
        assert!(eventually(|| titles("zed") == ["Zed"]));
        Ok(())
    }
}
