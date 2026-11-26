//! Web search with Kagi, for queries that start with a prefix such as `k `. Offers to search
//! kagi.com in the browser, which signs in with its own session, and with an API token in the
//! secret store also shows Kagi's results, fetched in the background once typing pauses.

use std::io::{self, Read};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use launcher_core::markdown::escape;
use launcher_core::{Action, Icon, Item, Plugin, ResultsChanged, Section, log};
use serde::Deserialize;
use url::Url;

pub const NAME: &str = "Kagi";
/// The API token's name in the secret store.
pub const SECRET: &str = "kagi";
const SEARCH_URL: &str = "https://kagi.com/search";
const API_URL: &str = "https://kagi.com/api/v1/search";
const TIMEOUT: Duration = Duration::from_secs(10);
/// As many as the panel shows without scrolling; the API returns no more.
const RESULTS: usize = 8;
/// The prefix says the query is for Kagi.
const RELEVANCE: u8 = 100;

/// Searches the API for the terms; swapped out in tests.
type Search = Arc<dyn Fn(&str) -> Outcome + Send + Sync>;

/// Starts the worker on a thread of its own; swapped out in tests.
type Spawn = fn(Box<dyn FnOnce() + Send>) -> io::Result<()>;

pub struct Kagi {
    prefix: String,
    shared: Arc<Shared>,
    search: Search,
    spawn: Spawn,
    /// The last worker couldn't start. Each keystroke tries again, but logs only the first
    /// failure in a row, as the log is kept on disk.
    cant_start: bool,
    /// How long typing must pause before a search goes to the API: each costs money and tells
    /// Kagi what was typed, so a half-typed query mustn't.
    pause: Duration,
}

/// Between the plugin, on the UI thread, and its worker.
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    changed: ResultsChanged,
}

#[derive(Default)]
struct State {
    /// The terms to search for once typing pauses, if any.
    wanted: Option<String>,
    /// Counts changes to `wanted`, so the worker can tell typing hasn't paused.
    typed: u64,
    /// The latest search's terms and how far it got. Only the latest, so responses for terms
    /// since changed are never shown.
    searched: Option<(String, Searched)>,
    /// A worker is waiting for a pause or searching. It starts when terms are wanted and exits
    /// once none are, so an idle plugin keeps no thread.
    working: bool,
    /// The plugin is gone.
    stopped: bool,
}

/// A result's row, and its detail, kept beside it until the row is opened.
type Row = (Item, Vec<Section>);

enum Searched {
    Pending,
    /// The rows that answer the terms, kept while they're the latest searched.
    Answered(Vec<Row>),
    /// Why the search failed, shown once: asking again searches again, as the network or token
    /// may have changed since.
    Failed(Item),
}

/// What a search found.
#[derive(Debug, PartialEq)]
enum Outcome {
    Results(Vec<Row>),
    NoToken,
    /// Why there are no results, and more about it.
    Problem(String, String),
}

impl Kagi {
    /// Searches for queries that start with `prefix` and a space, once typing pauses for
    /// `pause`.
    pub fn new(prefix: String, pause: Duration, changed: ResultsChanged) -> Self {
        Self::with(prefix, changed, Arc::new(search_api), pause)
    }

    fn with(prefix: String, changed: ResultsChanged, search: Search, pause: Duration) -> Self {
        Self {
            prefix,
            shared: Arc::new(Shared {
                state: Mutex::default(),
                wake: Condvar::new(),
                changed,
            }),
            search,
            spawn: spawn_worker,
            cant_start: false,
            pause,
        }
    }

    /// Asks the worker to search for `terms` once typing pauses, unless they're the latest
    /// searched, and returns the rows that answer them; `None` while they're being searched.
    /// `None` for terms stops a search that hasn't started.
    fn want(&mut self, terms: Option<&str>) -> Option<Vec<Item>> {
        let Ok(mut state) = self.shared.state.lock() else {
            return Some(vec![notice(
                "Kagi search stopped",
                "relaunch to search again",
            )]);
        };
        let latest = state
            .searched
            .as_ref()
            .filter(|(searched, _)| Some(searched.as_str()) == terms)
            .map(|(_, progress)| progress);
        let failed = matches!(latest, Some(Searched::Failed(_)));
        let (shown, settled) = match latest {
            None => (None, false),
            Some(Searched::Pending) => (None, true),
            Some(Searched::Answered(rows)) => (
                Some(rows.iter().map(|(item, _)| item.clone()).collect()),
                true,
            ),
            Some(Searched::Failed(row)) => (Some(vec![row.clone()]), true),
        };
        if failed {
            state.searched = None;
        }
        let wanted = terms.filter(|_| !settled);
        if state.wanted.as_deref() != wanted {
            state.wanted = wanted.map(str::to_owned);
            state.typed += 1;
            self.shared.wake.notify_one();
        }
        if wanted.is_some() && !state.working {
            let (shared, search, pause) = (self.shared.clone(), self.search.clone(), self.pause);
            match (self.spawn)(Box::new(move || work(&shared, &*search, pause))) {
                Ok(()) => {
                    state.working = true;
                    self.cant_start = false;
                }
                Err(error) => {
                    if !std::mem::replace(&mut self.cant_start, true) {
                        log!("can't start searching Kagi: {error}");
                    }
                    return Some(vec![notice("Can't search Kagi", &error.to_string())]);
                }
            }
        }
        shown
    }
}

impl Drop for Kagi {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.stopped = true;
            self.shared.wake.notify_one();
        }
    }
}

fn spawn_worker(work: Box<dyn FnOnce() + Send>) -> io::Result<()> {
    thread::Builder::new()
        .name("kagi search".into())
        .spawn(work)
        .map(drop)
}

/// Searches for the wanted terms each time typing pauses, until none are wanted.
fn work(shared: &Shared, search: &dyn Fn(&str) -> Outcome, pause: Duration) {
    let _unwinding = Unwinding(shared);
    while let Some(terms) = next_search(shared, pause) {
        let searched = match search(&terms) {
            Outcome::Results(rows) if rows.is_empty() => {
                Searched::Answered(vec![(notice("No results from Kagi", ""), Vec::new())])
            }
            Outcome::Results(rows) => Searched::Answered(rows),
            Outcome::NoToken => Searched::Failed(add_token()),
            Outcome::Problem(title, more) => Searched::Failed(notice(&title, &more)),
        };
        let Ok(mut state) = shared.state.lock() else {
            return;
        };
        state.searched = Some((terms, searched));
        // Unlocked, as the shell may ask for the results again at once.
        drop(state);
        (shared.changed)();
    }
}

/// Held by the worker so that, should a search panic, the worker is marked gone and its terms
/// unsearched, and the next terms wanted start another.
struct Unwinding<'a>(&'a Shared);

impl Drop for Unwinding<'_> {
    fn drop(&mut self) {
        if !thread::panicking() {
            return;
        }
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.working = false;
        if matches!(state.searched, Some((_, Searched::Pending))) {
            state.searched = None;
        }
    }
}

/// Waits for typing to pause with terms wanted, and marks them as being searched; `None`, with
/// the worker marked gone, once none are wanted or the plugin is gone.
fn next_search(shared: &Shared, pause: Duration) -> Option<String> {
    let mut state = shared.state.lock().ok()?;
    loop {
        if state.stopped || state.wanted.is_none() {
            state.working = false;
            return None;
        }
        let typed = state.typed;
        let (paused, waited) = shared
            .wake
            .wait_timeout_while(state, pause, |state| state.typed == typed && !state.stopped)
            .ok()?;
        state = paused;
        if waited.timed_out()
            && let Some(terms) = state.wanted.take()
        {
            state.searched = Some((terms.clone(), Searched::Pending));
            return Some(terms);
        }
    }
}

impl Plugin for Kagi {
    fn name(&self) -> &'static str {
        NAME
    }

    fn query(&mut self, query: &str) -> Vec<Item> {
        let terms = query
            .strip_prefix(self.prefix.as_str())
            .and_then(|rest| rest.strip_prefix(char::is_whitespace))
            .map(str::trim)
            .filter(|terms| !terms.is_empty());
        let Some(terms) = terms else {
            self.want(None);
            return Vec::new();
        };
        let rows = self
            .want(Some(terms))
            .unwrap_or_else(|| vec![notice("Searching Kagi…", "")]);
        [in_browser(terms)].into_iter().chain(rows).collect()
    }

    /// A result's snippet and address, kept from the search.
    fn detail(&mut self, _query: &str, item: &Item) -> Vec<Section> {
        let Ok(state) = self.shared.state.lock() else {
            return Vec::new();
        };
        match &state.searched {
            Some((_, Searched::Answered(rows))) => rows
                .iter()
                .find(|(row, _)| row.action == item.action)
                .map(|(_, detail)| detail.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn hidden(&mut self) {
        self.want(None);
    }
}

fn in_browser(terms: &str) -> Item {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("q", terms)
        .finish();
    item(
        format!("Search Kagi for “{terms}”"),
        "kagi.com".into(),
        Action::OpenUrl(format!("{SEARCH_URL}?{query}")),
    )
}

/// Says how to add a token, and copies the command that does.
fn add_token() -> Item {
    item(
        "Add a Kagi API token to see results here".into(),
        format!("Launcher secret set {SECRET}"),
        Action::Copy(launcher_secret::set_command(SECRET)),
    )
}

/// A muted row saying why there are no results.
fn notice(title: &str, subtitle: &str) -> Item {
    Item::notice(title, subtitle, Icon::Search, RELEVANCE)
}

fn item(title: String, subtitle: String, action: Action) -> Item {
    Item {
        title,
        subtitle,
        icon: Icon::Search,
        action,
        has_detail: false,
        relevance: RELEVANCE,
    }
}

/// Searches Kagi's API with the token from the secret store, read now so it isn't kept.
fn search_api(terms: &str) -> Outcome {
    let token = match launcher_secret::read(SECRET) {
        Ok(Some(token)) => token,
        Ok(None) => return Outcome::NoToken,
        Err(error) => {
            log!("can't read the Kagi token: {error}");
            return Outcome::Problem("Can't read the Kagi token".into(), error);
        }
    };
    let response = attohttpc::post(API_URL)
        .bearer_auth(token)
        .timeout(TIMEOUT)
        .json(&serde_json::json!({ "query": terms, "limit": RESULTS }))
        .and_then(|request| request.send());
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            // Not the query: the log is kept on disk.
            log!("can't reach Kagi: {error}");
            return Outcome::Problem("Can't reach Kagi".into(), error.to_string());
        }
    };
    let status = response.status().as_u16();
    let outcome = answer(status, response);
    if let Outcome::Problem(title, more) = &outcome {
        log!("{title} ({status}): {more}");
    }
    outcome
}

/// A successful response, with only what's shown.
#[derive(Deserialize)]
struct Found {
    data: Data,
}

#[derive(Deserialize)]
struct Data {
    #[serde(default)]
    search: Vec<ApiResult>,
}

#[derive(Deserialize)]
struct ApiResult {
    url: Option<String>,
    title: Option<String>,
    snippet: Option<String>,
}

/// An unsuccessful response.
#[derive(Deserialize)]
struct Refused {
    error: Vec<ApiError>,
}

#[derive(Deserialize)]
struct ApiError {
    code: Option<String>,
    message: Option<String>,
}

/// What a response from the API with HTTP `status` and `body` says.
fn answer(status: u16, body: impl Read) -> Outcome {
    if matches!(status, 401 | 403) {
        return Outcome::Problem(
            "Kagi rejected the API token".into(),
            format!("Launcher secret set {SECRET} replaces it"),
        );
    }
    if !(200..300).contains(&status) {
        let reason = serde_json::from_reader::<_, Refused>(body)
            .ok()
            .and_then(|failed| failed.error.into_iter().next())
            .and_then(|error| error.message.or(error.code))
            .unwrap_or_else(|| format!("HTTP {status}"));
        return Outcome::Problem("Kagi couldn't search".into(), reason);
    }
    match serde_json::from_reader::<_, Found>(body) {
        Ok(found) => Outcome::Results(
            found
                .data
                .search
                .into_iter()
                .filter_map(result_row)
                .collect(),
        ),
        Err(error) => Outcome::Problem("Kagi's answer can't be read".into(), error.to_string()),
    }
}

/// A result's row, for one with a web address; others, such as a `file:` URL, would open
/// something other than a page.
fn result_row(result: ApiResult) -> Option<Row> {
    let url = Url::parse(&result.url?).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let mut detail = Vec::new();
    if let Some(snippet) = result.snippet.as_deref().map(snippet_markdown)
        && !snippet.is_empty()
    {
        detail.push(Section::markdown("Snippet", snippet));
    }
    // A parsed URL has no spaces or angle brackets to end the autolink early.
    detail.push(Section::markdown("Address", format!("<{url}>")));
    let title = result
        .title
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| url.to_string());
    let host = url.host_str().unwrap_or_default().into();
    Some((
        Item {
            has_detail: true,
            ..item(title, host, Action::OpenUrl(url.into()))
        },
        detail,
    ))
}

/// Snippets come as HTML, with `<b>` around the words that matched: those stay bold, other tags
/// go, and the text is escaped.
fn snippet_markdown(html: &str) -> String {
    let mut snippet = Snippet::default();
    let mut rest = html;
    while let Some((before, from)) = rest
        .find(['<', '&'])
        .and_then(|index| rest.split_at_checked(index))
    {
        snippet.push(before);
        let tag = from
            .strip_prefix('<')
            // A tag starts `<b`, `</b` or `<!--`; `a < b` isn't one.
            .filter(|tag| {
                tag.starts_with(|first: char| first.is_ascii_alphabetic() || "/!".contains(first))
            })
            .and_then(|tag| tag.split_once('>'));
        rest = if let Some((tag, after)) = tag {
            let name = tag.split_whitespace().next().unwrap_or_default();
            match name.to_ascii_lowercase().as_str() {
                "b" | "strong" => snippet.start_bold(),
                "/b" | "/strong" => snippet.bold_closed = snippet.bold_from.is_some(),
                _ => {}
            }
            after
        } else if let Some((name, after)) =
            from.strip_prefix('&').and_then(|rest| rest.split_once(';'))
            && let Some(character) = entity(name)
        {
            snippet.push(character.encode_utf8(&mut [0; 4]));
            after
        } else {
            // Not markup, such as a lone `<` or `&`.
            let (character, after) = from.split_at_checked(1).unwrap_or((from, ""));
            snippet.push(character);
            after
        };
    }
    snippet.push(rest);
    snippet.end_bold();
    // Leading spaces could make it a code block.
    snippet.markdown.trim().into()
}

#[derive(Default)]
struct Snippet {
    markdown: String,
    /// Where the bold run being read starts in `markdown`.
    bold_from: Option<usize>,
    /// The run's tag closed, but it ends only once text follows, so that runs side by side
    /// join: `**a****b**` isn't bold.
    bold_closed: bool,
}

impl Snippet {
    fn push(&mut self, text: &str) {
        if self.bold_closed && !text.is_empty() {
            self.end_bold();
        }
        self.markdown.push_str(&escape(text));
    }

    fn start_bold(&mut self) {
        self.bold_closed = false;
        self.bold_from = self.bold_from.or(Some(self.markdown.len()));
    }

    /// Puts `**` around the bold run, inside any spaces at either end, as CommonMark needs;
    /// nothing if it's blank.
    fn end_bold(&mut self) {
        self.bold_closed = false;
        let Some(from) = self.bold_from.take() else {
            return;
        };
        let run = self.markdown.split_off(from);
        let bold = run.trim();
        if bold.is_empty() {
            self.markdown.push_str(&run);
            return;
        }
        self.markdown
            .push_str(run.strip_suffix(run.trim_start()).unwrap_or_default());
        self.markdown.push_str("**");
        self.markdown.push_str(bold);
        self.markdown.push_str("**");
        self.markdown
            .push_str(run.strip_prefix(run.trim_end()).unwrap_or_default());
    }
}

/// The character an HTML entity such as `amp` or `#39` stands for.
fn entity(name: &str) -> Option<char> {
    let named = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16),
                None => number.parse(),
            };
            return char::from_u32(code.ok()?);
        }
    };
    Some(named)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use launcher_core::Block;

    use super::*;

    /// A plugin whose searches answer with one hit titled with the terms, counting them, and
    /// a receiver told each time results change.
    fn plugin() -> (Kagi, Arc<Mutex<Vec<String>>>, mpsc::Receiver<()>) {
        let searches = Arc::new(Mutex::new(Vec::new()));
        let (sender, changed) = mpsc::channel();
        let recorded = searches.clone();
        let kagi = Kagi::with(
            "k".into(),
            Arc::new(move || {
                if let Err(error) = sender.send(()) {
                    panic!("{error}");
                }
            }),
            Arc::new(move |terms: &str| {
                if let Ok(mut searches) = recorded.lock() {
                    searches.push(terms.to_owned());
                }
                Outcome::Results(vec![(
                    item(terms.into(), String::new(), Action::Nothing),
                    Vec::new(),
                )])
            }),
            Duration::from_millis(50),
        );
        (kagi, searches, changed)
    }

    fn titles(kagi: &mut Kagi, query: &str) -> Vec<String> {
        kagi.query(query)
            .into_iter()
            .map(|item| item.title)
            .collect()
    }

    fn searched(searches: &Mutex<Vec<String>>) -> Vec<String> {
        searches
            .lock()
            .map(|searches| searches.clone())
            .unwrap_or_default()
    }

    #[test]
    fn only_queries_with_the_prefix_and_a_space_are_for_kagi() {
        let (mut kagi, searches, _) = plugin();
        for query in ["k", "kagi rust", "rust", "krust"] {
            assert!(kagi.query(query).is_empty(), "{query}");
        }
        assert!(
            kagi.shared.state.lock().is_ok_and(|state| !state.working),
            "no worker until a search"
        );
        assert_eq!(
            titles(&mut kagi, "k  rust lang"),
            ["Search Kagi for “rust lang”", "Searching Kagi…"]
        );
        assert!(searched(&searches).is_empty());
    }

    #[test]
    fn the_browser_row_opens_kagi_with_the_terms() {
        let item = in_browser("rust & c++");
        assert_eq!(
            item.action,
            Action::OpenUrl("https://kagi.com/search?q=rust+%26+c%2B%2B".into())
        );
    }

    #[test]
    fn searches_once_typing_pauses() -> Result<(), mpsc::RecvTimeoutError> {
        let (mut kagi, searches, changed) = plugin();
        for query in ["k r", "k ru", "k rus", "k rust"] {
            kagi.query(query);
        }
        changed.recv_timeout(Duration::from_secs(5))?;
        assert_eq!(searched(&searches), ["rust"]);
        assert_eq!(
            titles(&mut kagi, "k rust"),
            ["Search Kagi for “rust”", "rust"]
        );
        // Asking again, as the shell does when told the results changed, doesn't search again.
        kagi.query("k rust");
        assert!(changed.recv_timeout(Duration::from_millis(200)).is_err());
        assert_eq!(searched(&searches), ["rust"]);
        Ok(())
    }

    #[test]
    fn leaving_kagi_before_the_pause_searches_nothing() {
        let (mut kagi, searches, changed) = plugin();
        kagi.query("k rust");
        kagi.query("rust");
        assert!(changed.recv_timeout(Duration::from_millis(200)).is_err());
        assert!(searched(&searches).is_empty());
    }

    /// A plugin whose searches answer with `answer`, counting them, and a receiver told each
    /// time results change.
    fn plugin_answering(
        answer: impl Fn(&str) -> Outcome + Send + Sync + 'static,
    ) -> (Kagi, Arc<Mutex<Vec<String>>>, mpsc::Receiver<()>) {
        let searches = Arc::new(Mutex::new(Vec::new()));
        let (sender, changed) = mpsc::channel();
        let recorded = searches.clone();
        let kagi = Kagi::with(
            "k".into(),
            Arc::new(move || {
                if let Err(error) = sender.send(()) {
                    panic!("{error}");
                }
            }),
            Arc::new(move |terms: &str| {
                if let Ok(mut searches) = recorded.lock() {
                    searches.push(terms.to_owned());
                }
                answer(terms)
            }),
            Duration::from_millis(50),
        );
        (kagi, searches, changed)
    }

    #[test]
    fn hiding_or_clearing_before_the_pause_searches_nothing() {
        let (mut kagi, searches, changed) = plugin();
        kagi.query("k rust");
        kagi.hidden();
        assert!(changed.recv_timeout(Duration::from_millis(200)).is_err());
        kagi.query("k go");
        kagi.query("");
        assert!(changed.recv_timeout(Duration::from_millis(200)).is_err());
        assert!(searched(&searches).is_empty());
    }

    #[test]
    fn a_problem_shows_once_then_the_terms_are_searched_again() -> Result<(), mpsc::RecvTimeoutError>
    {
        let (mut kagi, searches, changed) =
            plugin_answering(|_| Outcome::Problem("Can't reach Kagi".into(), String::new()));
        kagi.query("k rust");
        changed.recv_timeout(Duration::from_secs(5))?;
        assert_eq!(
            titles(&mut kagi, "k rust"),
            ["Search Kagi for “rust”", "Can't reach Kagi"]
        );
        assert_eq!(
            titles(&mut kagi, "k rust"),
            ["Search Kagi for “rust”", "Searching Kagi…"]
        );
        changed.recv_timeout(Duration::from_secs(5))?;
        assert_eq!(searched(&searches), ["rust", "rust"]);
        Ok(())
    }

    #[test]
    fn a_search_that_panics_doesnt_stop_later_ones() -> Result<(), mpsc::RecvTimeoutError> {
        let (mut kagi, searches, changed) = plugin_answering(|terms| {
            assert_ne!(terms, "rust", "a search that panics");
            Outcome::Results(Vec::new())
        });
        kagi.query("k rust");
        // The worker panicked and is gone, untold.
        assert!(changed.recv_timeout(Duration::from_millis(300)).is_err());
        kagi.query("k go");
        changed.recv_timeout(Duration::from_secs(5))?;
        assert_eq!(searched(&searches), ["rust", "go"]);
        Ok(())
    }

    /// Runs itself again with stderr on a pipe, to count what it logs.
    #[test]
    fn a_worker_that_cant_start_is_logged_once() -> Result<(), Box<dyn std::error::Error>> {
        let Some(again) = launcher_core::run_test_again(
            "tests::a_worker_that_cant_start_is_logged_once",
            std::process::Stdio::piped(),
        )?
        else {
            let (mut kagi, _, _) = plugin();
            kagi.spawn = |_| Err(io::Error::other("no threads left"));
            for query in ["k w", "k we", "k web"] {
                assert_eq!(
                    titles(&mut kagi, query).get(1).map(String::as_str),
                    Some("Can't search Kagi")
                );
            }
            return Ok(());
        };
        let logged = String::from_utf8(again.stderr)?;
        assert_eq!(
            logged.matches("can't start searching Kagi").count(),
            1,
            "{logged}"
        );
        Ok(())
    }

    #[test]
    fn results_for_terms_since_changed_are_not_shown() -> Result<(), mpsc::RecvTimeoutError> {
        let (mut kagi, _, changed) = plugin();
        kagi.query("k rust");
        changed.recv_timeout(Duration::from_secs(5))?;
        assert_eq!(
            titles(&mut kagi, "k go"),
            ["Search Kagi for “go”", "Searching Kagi…"]
        );
        Ok(())
    }

    /// Each row's title, address and detail.
    fn rows(outcome: Outcome) -> Vec<(String, Action, Vec<Section>)> {
        let Outcome::Results(items) = outcome else {
            panic!("{outcome:?}");
        };
        items
            .into_iter()
            .map(|(item, detail)| (item.title, item.action, detail))
            .collect()
    }

    #[test]
    fn results_open_their_page_and_show_the_snippet_and_address() {
        let body = r#"{"meta": {}, "data": {"search": [
            {"url": "https://www.rust-lang.org/learn", "title": "Rust", "snippet": "A <b>language</b> &amp; more"},
            {"url": "file:///Applications/Calculator.app", "title": "Not a page"},
            {"url": "https://example.com/untitled", "title": " "},
            {"title": "No address"}
        ], "related_search": []}}"#;
        assert_eq!(
            rows(answer(200, body.as_bytes())),
            [
                (
                    "Rust".into(),
                    Action::OpenUrl("https://www.rust-lang.org/learn".into()),
                    vec![
                        Section::markdown("Snippet", r"A **language** \& more"),
                        Section::markdown("Address", "<https://www.rust-lang.org/learn>"),
                    ]
                ),
                (
                    "https://example.com/untitled".into(),
                    Action::OpenUrl("https://example.com/untitled".into()),
                    vec![Section::markdown(
                        "Address",
                        "<https://example.com/untitled>"
                    )]
                ),
            ]
        );
        assert_eq!(
            answer(200, r#"{"data": {"search": []}}"#.as_bytes()),
            Outcome::Results(Vec::new())
        );
    }

    #[test]
    fn failed_answers_say_what_went_wrong() {
        let problem = |status, body: &str| match answer(status, body.as_bytes()) {
            Outcome::Problem(title, more) => (title, more),
            outcome => panic!("{outcome:?}"),
        };
        assert_eq!(
            problem(401, "Unauthorized"),
            (
                "Kagi rejected the API token".into(),
                "Launcher secret set kagi replaces it".into()
            )
        );
        assert_eq!(
            problem(
                400,
                r#"{"meta": {}, "data": null, "error": [{"code": "billing.insufficient_credit", "url": "", "message": "Insufficient credit"}]}"#
            ),
            ("Kagi couldn't search".into(), "Insufficient credit".into())
        );
        assert_eq!(
            problem(502, "<html>Bad gateway</html>"),
            ("Kagi couldn't search".into(), "HTTP 502".into())
        );
        assert_eq!(problem(200, "<html>").0, "Kagi's answer can't be read");
    }

    #[test]
    fn snippets_keep_their_bold_and_lose_other_tags_and_entities() {
        // The text of each span, and whether it's bold.
        let read = |html| -> Vec<(String, bool)> {
            Section::markdown("Snippet", snippet_markdown(html))
                .blocks()
                .into_iter()
                .flat_map(|block| match block {
                    Block::Paragraph(spans) => spans,
                    block => panic!("{block:?}"),
                })
                .map(|span| (span.text, span.style.bold))
                .collect()
        };
        let plain = |text: &str| (text.to_string(), false);
        let bold = |text: &str| (text.to_string(), true);
        assert_eq!(
            read("<b>Rust</b> &lt;3 &amp; &quot;fast&quot; &#39;n&#x27; &nbsp;safe"),
            [bold("Rust"), plain(" <3 & \"fast\" 'n'  safe")]
        );
        assert_eq!(read("a < b && c; <i>d"), [plain("a < b && c; d")]);
        assert_eq!(read("unclosed <b"), [plain("unclosed <b")]);
        assert_eq!(
            read("*not* <B>left open"),
            [plain("*not* "), bold("left open")]
        );
        assert_eq!(
            read("a<b> spaced </b>b <b></b>c"),
            [plain("a "), bold("spaced"), plain(" b c")]
        );
        assert_eq!(read("<strong>two</strong><b>runs</b>"), [bold("tworuns")]);
        assert_eq!(read("    # indented"), [plain("# indented")]);
    }
}
