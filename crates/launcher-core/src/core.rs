use std::path::PathBuf;

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
}

/// Where settings and state live: `~/.config/launcher` on every platform.
pub fn config_directory() -> Option<PathBuf> {
    std::env::home_dir().map(|home| home.join(".config/launcher"))
}

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

/// The plugins a shell runs, asked in order.
pub struct Plugins(Vec<Box<dyn Plugin + Send>>);

impl Plugins {
    pub fn new(plugins: Vec<Box<dyn Plugin + Send>>) -> Self {
        Self(plugins)
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
        match self.0.get_mut(plugin.0) {
            Some(plugin) => plugin.ran(action),
            None => eprintln!("no plugin {} to tell it ran {action:?}", plugin.0),
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
}
