use std::cmp::Reverse;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use launcher_core::{Action, Icon, Item, Plugin, ResultsChanged, config_directory, log};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

pub const NAME: &str = "Apps";
const MAX_RESULTS: usize = 8;
const MAX_REMEMBERED: usize = 500;
/// Installs and updates touch the folders many times in a burst; rescan once it settles.
const SETTLE: Duration = Duration::from_millis(500);
/// A folder that never goes quiet (a helper logging next to its app) still gets rescanned.
const MAX_SETTLE: Duration = Duration::from_secs(3);
/// How often an app folder that doesn't exist yet is looked for.
const CHECK_MISSING: Duration = Duration::from_secs(5);

const MATCH: i32 = 16;
const WORD_START: i32 = 12;
/// Below `WORD_START`, so "ss" prefers System Settings to Chess.
const CONSECUTIVE: i32 = 4;
const GAP_START: i32 = 3;
const GAP_EXTEND: i32 = 1;
/// Two word starts' worth: frecency reorders close matches but can't overturn a much better one.
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
    /// The index thread holds it weakly, and stops once it's dropped. Each scan replaces it
    /// whole, so a panic while it's locked can't leave it half-written: it's used even poisoned.
    index: Arc<Mutex<Vec<App>>>,
    usage: HashMap<PathBuf, Usage>,
    saver: Option<Saver>,
    // Watching stops when this drops, which wakes the index thread waiting for changes.
    _watcher: Option<Arc<Mutex<RecommendedWatcher>>>,
}

impl Apps {
    /// Indexes the app folders in the background and keeps the index current, with frecency
    /// kept in the config directory. Calls `changed` when a scan changes the index.
    pub fn start(changed: ResultsChanged) -> Self {
        let frecency_path = config_directory().map(|directory| directory.join("frecency"));
        if frecency_path.is_none() {
            log!("no home folder, so apps are ranked without frecency");
        }
        Self::watching(app_folders(), frecency_path, changed)
    }

    /// Without a `frecency_path`, apps are ranked by match alone.
    fn watching(
        roots: Vec<PathBuf>,
        frecency_path: Option<PathBuf>,
        changed: ResultsChanged,
    ) -> Self {
        let roots: Vec<PathBuf> = roots.iter().map(|root| resolve(root)).collect();
        let index = Arc::new(Mutex::new(Vec::new()));
        let (sender, receiver) = mpsc::channel();
        let watcher = watch(sender).map(|watcher| Arc::new(Mutex::new(watcher)));
        thread::spawn({
            let index = Arc::downgrade(&index);
            let watcher = watcher.as_ref().map_or_else(Weak::new, Arc::downgrade);
            move || {
                keep_index(&index, &roots, &watcher, &receiver, &*changed);
            }
        });
        let usage = frecency_path.as_deref().map(load_usage).unwrap_or_default();
        let saver = frecency_path.map(Saver::start);
        Self {
            index,
            usage,
            saver,
            _watcher: watcher,
        }
    }
}

impl Plugin for Apps {
    fn name(&self) -> &'static str {
        NAME
    }

    fn query(&mut self, query: &str) -> Vec<Item> {
        let index = self.index.lock().unwrap_or_else(PoisonError::into_inner);
        rank(&index, &self.usage, query, now())
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(app, relevance)| Item {
                title: app.name.clone(),
                subtitle: app
                    .path
                    .parent()
                    .map(|folder| folder.display().to_string())
                    .unwrap_or_default(),
                icon: Icon::File(app.path.clone()),
                action: Action::Open(app.path.clone()),
                has_detail: false,
                relevance,
            })
            .collect()
    }

    fn ran(&mut self, action: &Action) {
        let (Action::Open(path), Some(saver)) = (action, &self.saver) else {
            return;
        };
        let now = now();
        let usage = self.usage.entry(path.to_path_buf()).or_default();
        usage.count = usage.count.saturating_add(1);
        usage.last_opened = now;
        // A loop: a loaded history can already be over the cap.
        while self.usage.len() > MAX_REMEMBERED
            && let Some(weakest) = self
                .usage
                .iter()
                .filter(|(remembered, _)| remembered.as_path() != path)
                .min_by(|left, right| left.1.weight(now).total_cmp(&right.1.weight(now)))
                .map(|(path, _)| path.clone())
        {
            self.usage.remove(&weakest);
        }
        saver.save(serialize_usage(&self.usage));
    }
}

/// Writes frecency on its own thread, in order, so a later save can't land before an earlier
/// one. Dropping it waits for the saves queued, so a plugin started next, as when settings turn
/// apps off and on again, reads them; each is a small file.
struct Saver {
    sender: Sender<String>,
    thread: Option<JoinHandle<()>>,
}

impl Saver {
    fn start(path: PathBuf) -> Self {
        let (sender, receiver) = mpsc::channel::<String>();
        let thread = thread::spawn(move || {
            while let Ok(contents) = receiver.recv() {
                // Each save is the whole history, so only the newest queued one needs writing;
                // a drop then waits for one write at most.
                let contents = receiver.try_iter().last().unwrap_or(contents);
                if let Err(error) = save(&path, &contents) {
                    log!("can't save {}: {error}", path.display());
                }
            }
        });
        Self {
            sender,
            thread: Some(thread),
        }
    }

    fn save(&self, contents: String) {
        if self.sender.send(contents).is_err() {
            log!("the frecency writer stopped; not saving");
        }
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        // Swapping in an unconnected sender closes the channel, which ends the thread once it
        // has written what's queued.
        self.sender = mpsc::channel().0;
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            log!("the frecency writer panicked");
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

/// Rescans after each burst of changes, until `index` or the watcher is dropped, when it returns
/// `None`. Each root is watched once it exists, before the scan, so an app added in between is
/// still caught.
fn keep_index(
    index: &Weak<Mutex<Vec<App>>>,
    roots: &[PathBuf],
    watcher: &Weak<Mutex<RecommendedWatcher>>,
    changes: &Receiver<()>,
    changed: &dyn Fn(),
) -> Option<()> {
    let mut watched = vec![false; roots.len()];
    // `None` once `index` is dropped.
    let mut refresh = || -> Option<()> {
        if let Some(watcher) = watcher.upgrade() {
            // Held only to add roots, which a panic adding another doesn't stop.
            let mut watcher = watcher.lock().unwrap_or_else(PoisonError::into_inner);
            for (root, watched) in roots.iter().zip(&mut watched) {
                if *watched || !root.is_dir() {
                    continue;
                }
                match watcher.watch(root, RecursiveMode::Recursive) {
                    Ok(()) => *watched = true,
                    Err(error) => log!("can't watch {}: {error}", root.display()),
                }
            }
        }
        // Scan before locking: `query` takes the same lock on every keystroke.
        let apps = scan(roots, &|| index.strong_count() == 0)?;
        let index = index.upgrade()?;
        let mut current = index.lock().unwrap_or_else(PoisonError::into_inner);
        // Updates write inside bundles in bursts that change nothing here, and every call
        // rebuilds the shown rows.
        if *current == apps {
            return Some(());
        }
        *current = apps;
        // Unlocked first: the shell asks for results again at once.
        drop(current);
        changed();
        Some(())
    };
    let missing = || -> Vec<bool> { roots.iter().map(|root| !root.is_dir()).collect() };
    let mut was_missing = missing();
    refresh()?;
    loop {
        // A root that doesn't exist yet (`~/Applications` often doesn't) is looked for now and
        // then. Watching its parent instead would wake for every write anywhere under home.
        let change = if was_missing.contains(&true) {
            changes.recv_timeout(CHECK_MISSING)
        } else {
            changes.recv().map_err(RecvTimeoutError::from)
        };
        match change {
            Ok(()) => {
                let deadline = Instant::now() + MAX_SETTLE;
                while Instant::now() < deadline && changes.recv_timeout(SETTLE).is_ok() {}
            }
            Err(RecvTimeoutError::Timeout) => {
                let appeared = was_missing
                    .iter()
                    .zip(roots)
                    .any(|(was_missing, root)| *was_missing && root.is_dir());
                if !appeared {
                    continue;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return None,
        }
        was_missing = missing();
        refresh()?;
    }
}

/// A watcher that tells `changes` of bundles appearing or going away; `keep_index` adds the roots.
fn watch(changes: Sender<()>) -> Option<RecommendedWatcher> {
    let handler = move |result: notify::Result<notify::Event>| match result {
        Ok(event) if event.paths.iter().any(|path| !inside_bundle(path)) => {
            if changes.send(()).is_err() {
                log!("the app index thread stopped; ignoring folder changes");
            }
        }
        Ok(_) => {}
        Err(error) => log!("app folder watcher: {error}"),
    };
    notify::recommended_watcher(handler)
        .inspect_err(|error| log!("can't watch app folders, so new apps need a restart: {error}"))
        .ok()
}

/// Resolves symlinks in the part of `path` that exists, since watchers report resolved paths
/// (`/private/var/…` for `/var/…`) and events are matched against the roots by prefix.
fn resolve(path: &Path) -> PathBuf {
    path.ancestors()
        .find_map(|ancestor| {
            let rest = path.strip_prefix(ancestor).ok()?;
            Some(fs::canonicalize(ancestor).ok()?.join(rest))
        })
        .unwrap_or_else(|| path.to_path_buf())
}

/// Apps write inside their own bundles (updates, caches); only bundles appearing or going
/// away change the index.
fn inside_bundle(path: &Path) -> bool {
    path.ancestors().skip(1).any(is_bundle)
}

fn is_bundle(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "app")
}

/// Finder's name for the app: localized, and often not the bundle's file name ("Find My" for
/// `FindMy.app`). Info.plist names don't match Finder's either, so ask the system.
#[cfg(target_os = "macos")]
fn localized_name(path: &Path) -> Option<String> {
    use std::ffi::c_void;
    use std::ptr::{self, NonNull};

    use objc2_core_foundation::{CFRetained, CFString, CFType, CFURL, kCFURLLocalizedNameKey};

    let url = CFURL::from_file_path(path)?;
    let mut name: *const CFType = ptr::null();
    // SAFETY: the key is CoreFoundation's own constant, and on success `name` holds a +1 retained
    // CFString, which `CFRetained::from_raw` takes ownership of.
    let found = unsafe {
        url.resource_property_for_key(
            kCFURLLocalizedNameKey,
            (&raw mut name).cast::<c_void>(),
            ptr::null_mut(),
        )
    };
    if !found {
        return None;
    }
    let name = unsafe { CFRetained::from_raw(NonNull::new(name.cast_mut())?) };
    let name = name.downcast::<CFString>().ok()?;
    // The name keeps `.app` when the user has chosen to show all extensions.
    let name = name.to_string();
    Some(name.strip_suffix(".app").map(str::to_owned).unwrap_or(name))
}

#[cfg(not(target_os = "macos"))]
fn localized_name(_path: &Path) -> Option<String> {
    None
}

/// `None` if `cancelled` says so between bundles.
fn scan(roots: &[PathBuf], cancelled: &dyn Fn() -> bool) -> Option<Vec<App>> {
    let mut apps = Vec::new();
    for root in roots {
        // One level of plain folders, for vendor folders such as `/Applications/Adobe …/`.
        collect_apps(root, 1, &mut apps, cancelled)?;
    }
    Some(distinguish(apps))
}

/// `apps` in name order, without the copies overlapping roots find twice. Finder can give
/// different bundles one name (`Siri.app` and `Siri AI.app` are both "Siri"); those take their
/// file names, so their rows differ. One app in several folders keeps Finder's name, as its
/// folders tell its rows apart.
fn distinguish(mut apps: Vec<App>) -> Vec<App> {
    apps.sort();
    apps.dedup();
    for alike in apps.chunk_by_mut(|left, right| left.name == right.name) {
        let mut file_names = alike.iter().map(|app| app.path.file_stem());
        let first = file_names.next();
        if file_names.all(|file_name| Some(file_name) == first) {
            continue;
        }
        for app in alike {
            if let Some(file_name) = app.path.file_stem() {
                app.name = file_name.to_string_lossy().into_owned();
            }
        }
    }
    apps.sort();
    apps
}

fn collect_apps(
    folder: &Path,
    depth: usize,
    apps: &mut Vec<App>,
    cancelled: &dyn Fn() -> bool,
) -> Option<()> {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Some(()),
        Err(error) => {
            log!("can't list {}: {error}", folder.display());
            return Some(());
        }
    };
    for entry in entries {
        if cancelled() {
            return None;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                log!("can't list {}: {error}", folder.display());
                continue;
            }
        };
        // Updaters stage hidden copies (`.Foo.app`) next to the real bundle.
        if entry.file_name().as_encoded_bytes().starts_with(b".") {
            continue;
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if is_bundle(&path)
            && let Some(name) = localized_name(&path)
                .or_else(|| Some(path.file_stem()?.to_string_lossy().into_owned()))
        {
            apps.push(App { name, path });
        } else if depth > 0 {
            collect_apps(&path, depth - 1, apps, cancelled)?;
        }
    }
    Some(())
}

/// Matching apps, best first, with their relevance. Ties keep the index's name order.
fn rank<'a>(
    apps: &'a [App],
    usage: &HashMap<PathBuf, Usage>,
    query: &str,
    now: u64,
) -> Vec<(&'a App, u8)> {
    let query = fold_query(query);
    if query.is_empty() {
        return Vec::new();
    }
    let mut scratch = Scratch::default();
    let mut ranked: Vec<(i32, &App, u8)> = apps
        .iter()
        .filter_map(|app| {
            let score = score(&app.name, &query, &mut scratch)?;
            let bonus = usage
                .get(&app.path)
                // Saturating float-to-int cast; the weight is never negative.
                .map_or(0, |usage| {
                    ((usage.weight(now).ln_1p() * 8.0) as i32).min(MAX_FRECENCY_BONUS)
                });
            // Frecency orders apps among themselves, but says nothing of how well the name
            // matches, which other plugins' results are weighed against.
            Some((score + bonus, app, relevance(score, query.len())))
        })
        .collect();
    ranked.sort_by_key(|(score, _, _)| Reverse(*score));
    ranked
        .into_iter()
        .map(|(_, app, relevance)| (app, relevance))
        .collect()
}

/// A score as a share of the best `score` gives a query of `length` letters, its start of a name
/// with nothing after: 100 for that, about 90 for the start of a short name, lower for gaps and
/// mid-word letters. Word starts past the first can push past 100; they're capped.
fn relevance(score: i32, length: usize) -> u8 {
    let consecutive = i32::try_from(length.saturating_sub(1)).unwrap_or(i32::MAX);
    let best = consecutive
        .saturating_mul(MATCH + CONSECUTIVE)
        .saturating_add(MATCH + WORD_START);
    u8::try_from((score.max(0).saturating_mul(100) / best).min(100)).unwrap_or(100)
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
fn score(name: &str, query: &[char], scratch: &mut Scratch) -> Option<i32> {
    let (first, rest) = query.split_first()?;
    // Most names don't match; this spares them the scoring below.
    if !contains_in_order(name, query) {
        return None;
    }
    let Scratch {
        folded,
        bonuses,
        best,
        next,
    } = scratch;
    folded.clear();
    bonuses.clear();
    let mut previous: Option<char> = None;
    for current in name.chars() {
        let word_start = previous.is_none_or(|previous| {
            !previous.is_alphanumeric() || (previous.is_lowercase() && current.is_uppercase())
        });
        folded.push(fold_case(current));
        bonuses.push(if word_start { WORD_START } else { 0 });
        previous = Some(current);
    }

    // `best[index]`: the best score with the current query letter matched at `name[index]`.
    best.clear();
    best.extend(
        folded
            .iter()
            .zip(bonuses.iter())
            .map(|(character, bonus)| (character == first).then_some(MATCH + bonus)),
    );
    for letter in rest {
        next.clear();
        next.resize(folded.len(), None);
        // The best earlier match with at least one letter skipped before `index`.
        let mut across_gap: Option<i32> = None;
        for index in 0..folded.len() {
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
        std::mem::swap(best, next);
    }
    // Only a tie-breaker between similar matches, so it stays small next to a word start.
    let length_penalty = i32::try_from(folded.len() / 2).unwrap_or(i32::MAX);
    best.iter()
        .flatten()
        .max()
        .map(|score| score.saturating_sub(length_penalty))
}

/// Buffers `score` reuses from one name to the next.
#[derive(Default)]
struct Scratch {
    folded: Vec<char>,
    bonuses: Vec<i32>,
    best: Vec<Option<i32>>,
    next: Vec<Option<i32>>,
}

/// Whether the folded `query`'s letters appear in `name` in order, the condition for `score`
/// to find a match.
fn contains_in_order(name: &str, query: &[char]) -> bool {
    let mut letters = name.chars().map(fold_case);
    query
        .iter()
        .all(|letter| letters.any(|character| character == *letter))
}

fn fold_case(character: char) -> char {
    character.to_lowercase().next().unwrap_or(character)
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
            log!("can't read {}: {error}", path.display());
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
                log!("skipping unreadable frecency line: {line:?}");
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
        .filter_map(|(path, usage)| {
            // A line-based file can't hold these; they'd be misread when loaded.
            let path = path.to_str().filter(|path| !path.contains('\n'))?;
            Some(format!("{}\t{}\t{path}\n", usage.count, usage.last_opened))
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
    use std::sync::atomic::{AtomicBool, Ordering};

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
            .map(|(app, _)| app.name.clone())
            .collect()
    }

    /// The calculator scores 90 for a calculation and 80 for a bare constant such as `pi`.
    #[test]
    fn relevance_is_a_share_of_the_best_match() {
        let installed = apps(&[
            "1Password",
            "Notes",
            "Pixelmator Pro",
            "Preview",
            "Safari",
            "Safe Exam Browser",
            "TextEdit",
            "Visual Studio Code",
        ]);
        let relevance = |query, name: &str| {
            rank(&installed, &HashMap::new(), query, 1_000_000)
                .into_iter()
                .find(|(app, _)| app.name == name)
                .map(|(_, relevance)| relevance)
        };
        assert_eq!(relevance("safari", "Safari"), Some(97));
        assert_eq!(relevance("saf", "Safari"), Some(95));
        assert_eq!(relevance("1pass", "1Password"), Some(96));
        assert_eq!(relevance("vsc", "Visual Studio Code"), Some(86));
        assert_eq!(relevance("pi", "Pixelmator Pro"), Some(85));
        assert_eq!(relevance("e", "TextEdit"), Some(85));
        assert_eq!(relevance("pi", "Preview"), Some(75), "a gap");
        assert_eq!(relevance("e", "Notes"), Some(50), "mid-word");
    }

    #[test]
    fn frecency_orders_apps_but_leaves_relevance_alone() {
        let installed = apps(&["Notes", "TextEdit"]);
        let usage = HashMap::from([(
            PathBuf::from("/Applications/Notes.app"),
            Usage {
                count: 10_000,
                last_opened: 1_000_000,
            },
        )]);
        let ranked: Vec<(&str, u8)> = rank(&installed, &usage, "e", 1_000_000)
            .into_iter()
            .map(|(app, relevance)| (app.name.as_str(), relevance))
            .collect();
        assert_eq!(
            ranked,
            [("Notes", 50), ("TextEdit", 85)],
            "a mid-word match stays below the calculator, however often it's opened"
        );
    }

    fn matches(name: &str, query: &str) -> Option<i32> {
        score(name, &fold_query(query), &mut Scratch::default())
    }

    fn titles(plugin: &mut Apps, query: &str) -> Vec<String> {
        plugin
            .query(query)
            .into_iter()
            .map(|item| item.title)
            .collect()
    }

    fn quiet() -> ResultsChanged {
        Arc::new(|| {})
    }

    fn eventually(condition: impl FnMut() -> bool) -> bool {
        eventually_within(Duration::from_secs(5), condition)
    }

    fn eventually_within(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
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
    fn scores_are_pinned() {
        let cases = [
            ("Safari", "saf", Some(65)),
            ("Safari", "safari", Some(125)),
            ("Safari", "sri", Some(56)),
            ("Safari", "fas", None),
            ("Safari", "x", None),
            ("Safe Exam Browser", "saf", Some(60)),
            ("Safe Exam Browser", "seb", Some(64)),
            ("Visual Studio Code", "vsc", Some(59)),
            ("Visual Studio Code", "studio code", Some(204)),
            ("Visual Studio Code", "vscode", Some(119)),
            ("System Settings", "ss", Some(41)),
            ("Screen Sharing", "ss", Some(41)),
            ("Chess", "ss", Some(34)),
            ("TextEdit", "e", Some(24)),
            ("TextEdit", "ted", Some(67)),
            ("Notes", "e", Some(14)),
            ("Preview", "pi", Some(36)),
            ("Pixelmator Pro", "pp", Some(37)),
            ("1Password", "1pass", Some(104)),
            ("zoom.us", "us", Some(45)),
            ("Annotate Hosts", "notes", Some(72)),
            ("Ünïcødé Äpp", "üä", Some(42)),
            ("İstanbul", "ist", Some(64)),
        ];
        for (name, query, expected) in cases {
            assert_eq!(matches(name, query), expected, "{query:?} in {name:?}");
        }
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
    fn opening_records_usage() -> io::Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("frecency");
        let mut plugin = Apps::watching(Vec::new(), Some(path.clone()), quiet());
        let safari = Path::new("/Applications/Safari.app");
        let open = Action::Open(safari.into());
        plugin.ran(&open);
        plugin.ran(&open);
        assert!(eventually(|| load_usage(&path)
            .get(safari)
            .is_some_and(|usage| usage.count == 2)));
        Ok(())
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn apps_take_finders_name() -> io::Result<()> {
        // Finder shows "Find My" or a translation, never the bundle's file name.
        let path = Path::new("/System/Applications/FindMy.app");
        let apps = scan_all(&[PathBuf::from("/System/Applications")])?;
        let name = apps
            .iter()
            .find(|app| app.path == path)
            .map(|app| &app.name);
        assert!(
            name.is_some_and(|name| name != "FindMy"),
            "FindMy.app scanned as {name:?}"
        );
        Ok(())
    }

    fn scan_all(roots: &[PathBuf]) -> io::Result<Vec<App>> {
        scan(roots, &|| false).ok_or_else(|| io::Error::other("a scan nothing cancelled stopped"))
    }

    #[test]
    fn scan_finds_bundles_one_folder_deep() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        for folder in [
            "Safari.app/Contents/Helper.app",
            "Vendor/Tool.app",
            "Vendor/Deeper/Hidden.app",
            "Not An App",
            ".Safari Update.app",
        ] {
            fs::create_dir_all(root.path().join(folder))?;
        }
        fs::write(root.path().join("notes.app"), "not a bundle")?;
        let names = |roots: &[PathBuf]| -> io::Result<Vec<String>> {
            Ok(scan_all(roots)?.into_iter().map(|app| app.name).collect())
        };
        let missing = root.path().join("missing");
        assert_eq!(
            names(&[root.path().to_path_buf(), missing])?,
            ["Safari", "Tool"]
        );
        // `/System/Applications/Utilities` is a root and inside one.
        let vendor = root.path().join("Vendor");
        assert_eq!(
            names(&[root.path().to_path_buf(), vendor])?,
            ["Hidden", "Safari", "Tool"]
        );
        assert_eq!(scan(&[root.path().to_path_buf()], &|| true), None);
        Ok(())
    }

    #[test]
    fn apps_finder_names_alike_take_their_file_names() {
        let app = |name: &str, path: &str| App {
            name: name.into(),
            path: path.into(),
        };
        let index = distinguish(vec![
            app("Siri", "/System/Applications/Siri.app"),
            app("Safari", "/Applications/Safari.app"),
            app("Siri", "/System/Applications/Siri AI.app"),
            app("Safari", "/Applications/Safari.app"),
            app("Zed Éditeur", "/Applications/Zed.app"),
            app("Zed Éditeur", "/Users/me/Applications/Zed.app"),
        ]);
        assert_eq!(
            index,
            [
                app("Safari", "/Applications/Safari.app"),
                app("Siri", "/System/Applications/Siri.app"),
                app("Siri AI", "/System/Applications/Siri AI.app"),
                app("Zed Éditeur", "/Applications/Zed.app"),
                app("Zed Éditeur", "/Users/me/Applications/Zed.app"),
            ],
            "an app found twice is one app, and copies in two folders keep Finder's name"
        );
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
    fn new_apps_appear_and_tell_the_shell() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("Safari.app"))?;
        let (sender, receiver) = mpsc::channel();
        let changed: ResultsChanged = Arc::new(move || {
            if sender.send(()).is_err() {
                log!("the test stopped listening");
            }
        });
        let mut plugin = Apps::watching(vec![root.path().to_path_buf()], None, changed);
        receiver.recv_timeout(Duration::from_secs(5))?;
        assert_eq!(titles(&mut plugin, "saf"), ["Safari"]);
        assert!(plugin.query("saf").iter().all(
            |item| matches!((&item.icon, &item.action), (Icon::File(icon), Action::Open(app)) if icon == app)
        ), "an app shows its own icon");
        fs::create_dir(root.path().join("Zed.app"))?;
        receiver.recv_timeout(Duration::from_secs(10))?;
        assert_eq!(titles(&mut plugin, "zed"), ["Zed"]);
        Ok(())
    }

    #[test]
    fn a_panic_holding_the_index_loses_no_apps() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("Safari.app"))?;
        let mut plugin = Apps::watching(vec![root.path().to_path_buf()], None, quiet());
        assert!(eventually(|| titles(&mut plugin, "saf") == ["Safari"]));
        let held = std::panic::catch_unwind(|| {
            let _held = plugin.index.lock();
            panic!("panicking with the index locked");
        });
        assert!(held.is_err() && plugin.index.is_poisoned());
        assert_eq!(titles(&mut plugin, "saf"), ["Safari"]);
        fs::create_dir(root.path().join("Zed.app"))?;
        assert!(
            eventually(|| titles(&mut plugin, "zed") == ["Zed"]),
            "rescans still replace the index"
        );
        Ok(())
    }

    #[test]
    fn turning_apps_off_stops_its_scan() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        // Far more than scan in the second the thread gets to end.
        for index in 0..2000 {
            fs::create_dir(root.path().join(format!("{index}.app")))?;
        }
        let called = Arc::new(AtomicBool::new(false));
        let changed: ResultsChanged = Arc::new({
            let called = called.clone();
            move || called.store(true, Ordering::Relaxed)
        });
        // The index thread holds the only other reference.
        let running = Arc::downgrade(&changed);
        drop(Apps::watching(
            vec![root.path().to_path_buf()],
            None,
            changed,
        ));
        assert!(
            eventually_within(Duration::from_secs(1), || running.strong_count() == 0),
            "the index thread ends"
        );
        assert!(!called.load(Ordering::Relaxed), "nothing to refresh");
        Ok(())
    }

    #[test]
    fn app_folders_created_later_are_watched() -> io::Result<()> {
        let home = tempfile::tempdir()?;
        let applications = home.path().join("Applications");
        let mut plugin = Apps::watching(vec![applications.clone()], None, quiet());
        fs::create_dir_all(applications.join("Zed.app"))?;
        let found = eventually_within(CHECK_MISSING * 2, || titles(&mut plugin, "zed") == ["Zed"]);
        assert!(found);
        fs::create_dir(applications.join("Yak.app"))?;
        assert!(eventually(|| titles(&mut plugin, "yak") == ["Yak"]));
        Ok(())
    }

    #[test]
    fn constant_writes_dont_hold_off_rescans() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let vendor = root.path().join("Vendor");
        fs::create_dir(&vendor)?;
        fs::create_dir(root.path().join("Safari.app"))?;
        let mut plugin = Apps::watching(vec![root.path().to_path_buf()], None, quiet());
        assert!(eventually(|| titles(&mut plugin, "saf") == ["Safari"]));
        let stop = Arc::new(AtomicBool::new(false));
        let writer = thread::spawn({
            let stop = stop.clone();
            move || -> io::Result<()> {
                while !stop.load(Ordering::Relaxed) {
                    fs::write(vendor.join("log"), "busy")?;
                    thread::sleep(Duration::from_millis(100));
                }
                Ok(())
            }
        });
        fs::create_dir(root.path().join("Zed.app"))?;
        let found = eventually(|| titles(&mut plugin, "zed") == ["Zed"]);
        stop.store(true, Ordering::Relaxed);
        assert!(writer.join().is_ok_and(|result| result.is_ok()));
        assert!(found);
        Ok(())
    }

    #[test]
    fn opening_trims_an_overfull_history_to_the_cap() -> io::Result<()> {
        let folder = tempfile::tempdir()?;
        let mut plugin = Apps::watching(Vec::new(), Some(folder.path().join("frecency")), quiet());
        plugin.usage = (0..MAX_REMEMBERED * 2)
            .map(|index| {
                let path = PathBuf::from(format!("/Applications/{index}.app"));
                (path, Usage::default())
            })
            .collect();
        plugin.ran(&Action::Open("/Applications/Safari.app".into()));
        assert_eq!(plugin.usage.len(), MAX_REMEMBERED);
        Ok(())
    }

    #[test]
    fn opening_a_new_app_when_full_keeps_it() -> io::Result<()> {
        let folder = tempfile::tempdir()?;
        let mut plugin = Apps::watching(Vec::new(), Some(folder.path().join("frecency")), quiet());
        let now = now();
        plugin.usage = (0..MAX_REMEMBERED)
            .map(|index| {
                let usage = Usage {
                    count: 2,
                    last_opened: now,
                };
                (PathBuf::from(format!("/Applications/{index}.app")), usage)
            })
            .collect();
        let safari = Path::new("/Applications/Safari.app");
        plugin.ran(&Action::Open(safari.into()));
        assert!(plugin.usage.contains_key(safari));
        assert_eq!(plugin.usage.len(), MAX_REMEMBERED);
        Ok(())
    }

    #[test]
    fn a_restarted_plugin_sees_the_last_open_before_it() -> io::Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("frecency");
        let safari = PathBuf::from("/Applications/Safari.app");
        // As when settings turn apps off and straight back on.
        for opens in 1..=20 {
            let mut plugin = Apps::watching(Vec::new(), Some(path.clone()), quiet());
            plugin.ran(&Action::Open(safari.clone()));
            drop(plugin);
            let count = load_usage(&path).get(&safari).map(|usage| usage.count);
            assert_eq!(count, Some(opens));
        }
        Ok(())
    }

    #[test]
    fn newlines_in_paths_dont_corrupt_frecency() {
        let safari = PathBuf::from("/Applications/Safari.app");
        let usage = Usage {
            count: 1,
            last_opened: 7,
        };
        let saved = serialize_usage(&HashMap::from([
            (safari.clone(), usage),
            (PathBuf::from("/Applications/Two\nLines.app"), usage),
        ]));
        assert_eq!(parse_usage(&saved), HashMap::from([(safari, usage)]));
    }
}
