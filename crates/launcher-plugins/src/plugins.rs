//! The plugins every shell runs and the settings that choose them, so a shell only draws
//! results, registers hotkeys and runs actions.

use std::time::Duration;

use launcher_apps::Apps;
use launcher_calc::Calc;
use launcher_config::Config;
use launcher_core::{Group, Plugin, Plugins, ResultsChanged, Selection, cache_directory, log};
use launcher_jwt::Jwt;
use launcher_kagi::Kagi;

/// The secrets plugins read, which the `secret` command sets.
const SECRETS: [&str; 1] = [launcher_kagi::SECRET];

/// Runs the commands every shell shares, such as `config init` and `secret set kagi`, before the
/// shell starts its UI: `None` when `arguments` aren't one, else the exit status.
pub fn run_command(arguments: &[String]) -> Option<i32> {
    launcher_config::run_command(arguments)
        .or_else(|| launcher_secret::run_command(arguments, &SECRETS))
}

/// The plugins, in the order their results show when equally relevant; each has a place in
/// `Plugins`, empty while it's off.
#[derive(Clone, Copy)]
enum Place {
    Calculator,
    Jwt,
    Apps,
    Kagi,
}

const PLACES: [Place; 4] = [Place::Calculator, Place::Jwt, Place::Apps, Place::Kagi];

/// Everything a plugin reads from the settings, and all `start` has to go on, so a plugin
/// restarts whenever any of it changes.
#[derive(PartialEq)]
enum PluginSettings {
    Calculator,
    Jwt,
    Apps,
    Kagi(launcher_config::Kagi),
}

impl Place {
    /// Starts the plugin `config` turns on in this place, which is `index` in `plugins`, or
    /// empties the place while it's off.
    fn start(self, plugins: &mut Plugins, index: usize, config: &Config, changed: &ResultsChanged) {
        match self.settings(config) {
            Some(settings) => plugins.start(index, self.name(), || settings.start(changed)),
            None => plugins.turn_off(index),
        }
    }

    /// The plugin's `Plugin::name`, for before it has started.
    fn name(self) -> &'static str {
        match self {
            Self::Calculator => launcher_calc::NAME,
            Self::Jwt => launcher_jwt::NAME,
            Self::Apps => launcher_apps::NAME,
            Self::Kagi => launcher_kagi::NAME,
        }
    }

    /// What the plugin in this place reads from `config`; `None` while it's off.
    fn settings(self, config: &Config) -> Option<PluginSettings> {
        let enabled = config.plugins;
        match self {
            Self::Calculator => enabled.calculator.then_some(PluginSettings::Calculator),
            Self::Jwt => enabled.jwt.then_some(PluginSettings::Jwt),
            Self::Apps => enabled.apps.then_some(PluginSettings::Apps),
            Self::Kagi => enabled
                .kagi
                .then(|| PluginSettings::Kagi(config.kagi.clone())),
        }
    }
}

impl PluginSettings {
    /// Starts the plugin, and its background work, which calls `changed` when it may have
    /// changed the results.
    fn start(self, changed: &ResultsChanged) -> Box<dyn Plugin + Send> {
        match self {
            Self::Calculator => {
                let rates_directory = cache_directory().unwrap_or_else(|| {
                    log!("no home folder, so exchange rates are cached in the temporary folder");
                    std::env::temp_dir()
                });
                Box::new(Calc::start(rates_directory, changed.clone()))
            }
            Self::Jwt => Box::new(Jwt),
            Self::Apps => Box::new(Apps::start(changed.clone())),
            Self::Kagi(settings) => Box::new(Kagi::new(
                settings.prefix,
                Duration::from_millis(settings.pause_ms),
                changed.clone(),
            )),
        }
    }
}

/// The plugins and the settings from the config file, kept as the file changes, and which of
/// the results shown is picked.
pub struct Launcher {
    plugins: Plugins,
    selection: Selection,
    config: Config,
    /// Why the config file can't be used; `config` is then the settings from before, or the
    /// defaults.
    problem: Option<String>,
    /// Handed to plugins restarted when the settings change.
    results_changed: ResultsChanged,
}

impl Launcher {
    /// Starts the plugins the config file turns on. The file is read again only on `reload`.
    pub fn start(results_changed: ResultsChanged) -> Self {
        let (config, problem) = match launcher_config::load() {
            Ok(config) => (config, None),
            Err(problem) => (Config::default(), Some(problem)),
        };
        let mut plugins = Plugins::empty(PLACES.len());
        for (index, place) in PLACES.into_iter().enumerate() {
            place.start(&mut plugins, index, &config, &results_changed);
        }
        Self::with(plugins, config, problem, results_changed)
    }

    pub fn with(
        plugins: Plugins,
        config: Config,
        problem: Option<String>,
        results_changed: ResultsChanged,
    ) -> Self {
        Self {
            plugins,
            selection: Selection::default(),
            config,
            problem,
            results_changed,
        }
    }

    pub fn plugins(&mut self) -> &mut Plugins {
        &mut self.plugins
    }

    /// The results for `query`, to show, and the index of the item picked among them.
    pub fn query(&mut self, query: &str) -> (Vec<Group>, usize) {
        let groups = self.plugins.query(query);
        let picked = self.selection.show(query, &groups);
        (groups, picked)
    }

    /// The user pointed at the `index`th item shown or opened its detail; returns the item
    /// picked.
    pub fn pick(&mut self, index: usize) -> usize {
        self.selection.pick(index)
    }

    /// ↑, wrapping to the last item; returns the item picked.
    pub fn up(&mut self) -> usize {
        self.selection.up()
    }

    /// ↓, wrapping to the first item; returns the item picked.
    pub fn down(&mut self) -> usize {
        self.selection.down()
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// Reads the config file again, and restarts plugins that crashed; whether the settings or
    /// their problem changed.
    pub fn reload(&mut self) -> bool {
        self.apply(launcher_config::load())
    }

    /// Takes `loaded` settings, starting or dropping only the plugins they turn on or off, or
    /// whose settings they change, and restarting those that crashed. On a problem, keeps the
    /// settings from before; the problem always counts as a change, so the shell says it again.
    fn apply(&mut self, loaded: Result<Config, String>) -> bool {
        let (config, problem) = match loaded {
            Ok(config) => (config, None),
            Err(problem) => (self.config.clone(), Some(problem)),
        };
        let changed = problem.is_some() || self.problem.is_some() || config != self.config;
        let mut restarted = false;
        for (index, place) in PLACES.into_iter().enumerate() {
            if place.settings(&config) != place.settings(&self.config)
                || self.plugins.crashed(index)
            {
                place.start(&mut self.plugins, index, &config, &self.results_changed);
                restarted = true;
            }
        }
        if restarted {
            (self.results_changed)();
        }
        self.config = config;
        self.problem = problem;
        changed
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use launcher_config::EnabledPlugins;
    use launcher_core::Item;

    use super::*;

    struct Echo;

    impl Plugin for Echo {
        fn name(&self) -> &'static str {
            "Echo"
        }

        fn query(&mut self, query: &str) -> Vec<Item> {
            vec![Item {
                title: query.into(),
                subtitle: String::new(),
                icon: launcher_core::Icon::Calculator,
                action: launcher_core::Action::Nothing,
                detail: Vec::new(),
                relevance: 50,
            }]
        }
    }

    fn launcher() -> Launcher {
        Launcher::with(
            Plugins::new(vec![Box::new(Echo)]),
            Config::default(),
            None,
            Arc::new(|| {}),
        )
    }

    #[test]
    fn a_problem_keeps_the_settings_from_before() {
        let mut launcher = launcher();
        let unbound = Config {
            window: Vec::new(),
            ..Config::default()
        };
        assert!(launcher.apply(Ok(unbound.clone())));
        assert!(launcher.apply(Err("config.toml: nonsense".into())));
        assert_eq!(launcher.config(), &unbound);
        assert_eq!(launcher.problem(), Some("config.toml: nonsense"));
        // The same plugins are on, so they weren't restarted.
        assert_eq!(launcher.plugins().query("anything").len(), 1);
    }

    #[test]
    fn the_picked_item_stays_with_the_user_across_refreshes() {
        let mut launcher = Launcher::with(
            Plugins::new(vec![Box::new(Echo), Box::new(Echo)]),
            Config::default(),
            None,
            Arc::new(|| {}),
        );
        assert_eq!(launcher.query("a").1, 0);
        assert_eq!(launcher.down(), 1);
        assert_eq!(launcher.query("a").1, 1, "a refresh keeps it");
        assert_eq!(launcher.up(), 0);
        assert_eq!(launcher.pick(1), 1);
        assert_eq!(launcher.query("ab").1, 0, "a new query starts at the top");
    }

    /// The ids of the plugins with results for `query`.
    fn answering(launcher: &mut Launcher, query: &str) -> Vec<String> {
        launcher
            .plugins()
            .query(query)
            .into_iter()
            .map(|group| format!("{:?}", group.plugin))
            .collect()
    }

    #[test]
    fn turning_a_plugin_off_keeps_the_others() {
        let mut launcher = Launcher::with(
            Plugins::new(vec![Box::new(Echo), Box::new(Echo), Box::new(Echo)]),
            Config::default(),
            None,
            Arc::new(|| {}),
        );
        let jwt_off = Config {
            plugins: EnabledPlugins {
                jwt: false,
                ..EnabledPlugins::default()
            },
            ..Config::default()
        };
        assert!(launcher.apply(Ok(jwt_off)));
        assert_eq!(
            answering(&mut launcher, "anything"),
            ["PluginId(0)", "PluginId(2)"]
        );
    }

    #[test]
    fn a_plugin_restarts_when_its_own_settings_change() {
        let mut launcher = Launcher::with(
            Plugins::new(vec![
                Box::new(Echo),
                Box::new(Echo),
                Box::new(Echo),
                Box::new(Echo),
            ]),
            Config::default(),
            None,
            Arc::new(|| {}),
        );
        let mut kagi_prefix = Config::default();
        kagi_prefix.kagi.prefix = "kagi".into();
        assert!(launcher.apply(Ok(kagi_prefix)));
        assert_eq!(
            answering(&mut launcher, "anything"),
            ["PluginId(0)", "PluginId(1)", "PluginId(2)"],
            "Kagi, in place of the last echo, answers only its prefix"
        );
    }

    struct Panicking;

    impl Plugin for Panicking {
        fn name(&self) -> &'static str {
            "Panicking"
        }

        fn query(&mut self, _query: &str) -> Vec<Item> {
            panic!("a bug in query");
        }
    }

    #[test]
    fn reloading_restarts_a_crashed_plugin() {
        let refreshes = Arc::new(AtomicUsize::new(0));
        let counter = refreshes.clone();
        let mut launcher = Launcher::with(
            Plugins::new(vec![
                Box::new(Echo),
                Box::new(Panicking),
                Box::new(Echo),
                Box::new(Echo),
            ]),
            Config::default(),
            None,
            Arc::new(move || {
                counter.fetch_add(1, Ordering::Relaxed);
            }),
        );
        launcher.query("anything");
        assert!(launcher.plugins().crashed(1));
        assert!(
            !launcher.apply(Ok(Config::default())),
            "the settings are the same"
        );
        assert!(!launcher.plugins().crashed(1));
        assert_eq!(refreshes.load(Ordering::Relaxed), 1);
        assert_eq!(
            answering(&mut launcher, "not a token"),
            ["PluginId(0)", "PluginId(2)", "PluginId(3)"],
            "the token plugin is back, answering only tokens"
        );
    }

    #[test]
    fn unchanged_settings_are_no_change() {
        let mut launcher = launcher();
        assert!(!launcher.apply(Ok(Config::default())));
        assert!(launcher.apply(Err("config.toml: nonsense".into())));
        assert!(launcher.apply(Ok(Config::default())));
        assert_eq!(launcher.problem(), None);
        assert!(!launcher.apply(Ok(Config::default())));
    }
}
