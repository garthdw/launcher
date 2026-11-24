//! The plugins every shell runs and the settings that choose them, so a shell only draws
//! results, registers hotkeys and runs actions.

use launcher_apps::Apps;
use launcher_calc::Calc;
use launcher_config::{Config, EnabledPlugins};
use launcher_core::{Plugin, Plugins, ResultsChanged, cache_directory, log};
use launcher_jwt::Jwt;
use notify::RecommendedWatcher;

/// The plugins, in the order their results show when equally relevant; each has a place in
/// `Plugins`, empty while it's off.
#[derive(Clone, Copy)]
enum Place {
    Calculator,
    Jwt,
    Apps,
}

const PLACES: [Place; 3] = [Place::Calculator, Place::Jwt, Place::Apps];

impl Place {
    fn enabled(self, enabled: EnabledPlugins) -> bool {
        match self {
            Self::Calculator => enabled.calculator,
            Self::Jwt => enabled.jwt,
            Self::Apps => enabled.apps,
        }
    }

    /// Starts the plugin's background work, which calls `changed` when it may have changed the
    /// results.
    fn start(self, changed: &ResultsChanged) -> Option<Box<dyn Plugin + Send>> {
        match self {
            Self::Calculator => {
                let rates_directory = cache_directory().unwrap_or_else(|| {
                    log!("no home folder, so exchange rates are cached in the temporary folder");
                    std::env::temp_dir()
                });
                match Calc::start(rates_directory, changed.clone()) {
                    Ok(calc) => Some(Box::new(calc)),
                    Err(error) => {
                        log!("calculator unavailable: {error}");
                        None
                    }
                }
            }
            Self::Jwt => Some(Box::new(Jwt)),
            Self::Apps => Some(Box::new(Apps::start(changed.clone()))),
        }
    }
}

/// The plugins and the settings from the config file, kept as the file changes.
pub struct Launcher {
    plugins: Plugins,
    config: Config,
    /// Why the config file can't be used; `config` is then the settings from before, or the
    /// defaults.
    problem: Option<String>,
    /// Handed to plugins restarted when the settings turn them on or off.
    results_changed: ResultsChanged,
    /// Watching the config file stops when this drops.
    _watcher: Option<RecommendedWatcher>,
}

impl Launcher {
    /// Starts the plugins the config file turns on, and calls `settings_changed` from another
    /// thread whenever the file changes, for the shell to call `reload`.
    pub fn start(
        results_changed: ResultsChanged,
        settings_changed: impl Fn() + Send + 'static,
    ) -> Self {
        // Watching before reading, so a save in between still reloads.
        let watched = launcher_config::path().and_then(|path| {
            launcher_config::watch(&path, settings_changed).map_err(|error| {
                format!(
                    "can't watch {}, so changes need a relaunch: {error}",
                    path.display()
                )
            })
        });
        let (config, load_problem) = match launcher_config::load() {
            Ok(config) => (config, None),
            Err(problem) => (Config::default(), Some(problem)),
        };
        let (watcher, watch_problem) = match watched {
            Ok(watcher) => (Some(watcher), None),
            Err(problem) => (None, Some(problem)),
        };
        let problems: Vec<String> = load_problem.into_iter().chain(watch_problem).collect();
        let problem = (!problems.is_empty()).then(|| problems.join("\n\n"));
        let mut plugins = Plugins::empty(PLACES.len());
        for (index, place) in PLACES.into_iter().enumerate() {
            if place.enabled(config.plugins) {
                plugins.set(index, place.start(&results_changed));
            }
        }
        Self {
            _watcher: watcher,
            ..Self::with(plugins, config, problem, results_changed)
        }
    }

    /// Without watching the config file.
    pub fn with(
        plugins: Plugins,
        config: Config,
        problem: Option<String>,
        results_changed: ResultsChanged,
    ) -> Self {
        Self {
            plugins,
            config,
            problem,
            results_changed,
            _watcher: None,
        }
    }

    pub fn plugins(&mut self) -> &mut Plugins {
        &mut self.plugins
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// Reads the config file again; whether the settings or their problem changed.
    pub fn reload(&mut self) -> bool {
        self.apply(launcher_config::load())
    }

    /// Takes `loaded` settings, starting or dropping only the plugins they turn on or off. On a
    /// problem, keeps the settings from before; the problem always counts as a change, so the
    /// shell says it again.
    fn apply(&mut self, loaded: Result<Config, String>) -> bool {
        let config = match loaded {
            Ok(config) => config,
            Err(problem) => {
                self.problem = Some(problem);
                return true;
            }
        };
        if config == self.config && self.problem.is_none() {
            return false;
        }
        if config.plugins != self.config.plugins {
            let toggled = PLACES.into_iter().enumerate().filter(|(_, place)| {
                place.enabled(config.plugins) != place.enabled(self.config.plugins)
            });
            for (index, place) in toggled {
                let plugin = place
                    .enabled(config.plugins)
                    .then(|| place.start(&self.results_changed))
                    .flatten();
                self.plugins.set(index, plugin);
            }
            (self.results_changed)();
        }
        self.config = config;
        self.problem = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

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
        let places: Vec<String> = launcher
            .plugins()
            .query("anything")
            .into_iter()
            .map(|group| format!("{:?}", group.plugin))
            .collect();
        assert_eq!(places, ["PluginId(0)", "PluginId(2)"]);
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
