//! The boundary the native shells call. Its types mirror `launcher-core`'s so the core stays
//! free of FFI derives; a Rust shell (Linux) uses the core directly.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use launcher_apps::Apps;
use launcher_calc::Calc;
use launcher_core::Plugin;

uniffi::setup_scaffolding!();

#[derive(uniffi::Record)]
pub struct Item {
    pub title: String,
    pub subtitle: String,
    pub action: Action,
    /// Which plugin produced the item, so `Launcher::ran` reaches only that one.
    pub plugin: u32,
}

#[derive(uniffi::Enum)]
pub enum Action {
    Copy { text: String },
    Open { path: String },
}

impl From<launcher_core::Action> for Action {
    fn from(action: launcher_core::Action) -> Self {
        match action {
            launcher_core::Action::Copy(text) => Self::Copy { text },
            launcher_core::Action::Open(path) => Self::Open {
                path: path.to_string_lossy().into_owned(),
            },
        }
    }
}

impl From<Action> for launcher_core::Action {
    fn from(action: Action) -> Self {
        match action {
            Action::Copy { text } => Self::Copy(text),
            Action::Open { path } => Self::Open(PathBuf::from(path)),
        }
    }
}

#[derive(uniffi::Object)]
pub struct Launcher {
    plugins: Mutex<Vec<Box<dyn Plugin + Send>>>,
}

#[uniffi::export]
impl Launcher {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        let mut plugins: Vec<Box<dyn Plugin + Send>> = Vec::new();
        match Calc::new() {
            Ok(calc) => plugins.push(Box::new(calc)),
            Err(error) => eprintln!("calculator unavailable: {error}"),
        }
        plugins.push(Box::new(Apps::start()));
        Arc::new(Self {
            plugins: Mutex::new(plugins),
        })
    }

    /// Called on every keystroke from the UI thread, so plugins must answer well inside a frame.
    pub fn query(&self, query: String) -> Vec<Item> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }
        let Ok(mut plugins) = self.plugins.lock() else {
            eprintln!("a plugin panicked; skipping queries");
            return Vec::new();
        };
        (0..)
            .zip(plugins.iter_mut())
            .flat_map(|(index, plugin)| {
                plugin.query(query).into_iter().map(move |item| Item {
                    title: item.title,
                    subtitle: item.subtitle,
                    action: item.action.into(),
                    plugin: index,
                })
            })
            .collect()
    }

    /// The shell ran `item`'s action successfully.
    pub fn ran(&self, item: Item) {
        let Ok(mut plugins) = self.plugins.lock() else {
            eprintln!("a plugin panicked; not telling it {} ran", item.title);
            return;
        };
        let Some(plugin) = usize::try_from(item.plugin)
            .ok()
            .and_then(|index| plugins.get_mut(index))
        else {
            eprintln!("no plugin {} for {}", item.plugin, item.title);
            return;
        };
        plugin.ran(&item.action.into());
    }
}

/// Fetches exchange rates on a background thread, so the first currency query doesn't block.
#[uniffi::export]
pub fn prefetch_exchange_rates() {
    std::thread::spawn(Calc::prefetch_exchange_rates);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_converts_items() -> Result<(), Box<dyn std::error::Error>> {
        // Only the calculator, so installed apps can't add results.
        let launcher = Launcher {
            plugins: Mutex::new(vec![Box::new(Calc::new()?)]),
        };
        let items = launcher.query(" 2 + 3 ".into());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "5");
        assert!(matches!(&items[0].action, Action::Copy { text } if text == "5"));
        assert!(launcher.query("   ".into()).is_empty());
        Ok(())
    }

    struct Recorder {
        title: &'static str,
        ran: Arc<Mutex<Vec<launcher_core::Action>>>,
    }

    impl Plugin for Recorder {
        fn query(&mut self, _query: &str) -> Vec<launcher_core::Item> {
            vec![launcher_core::Item {
                title: self.title.into(),
                subtitle: String::new(),
                action: launcher_core::Action::Copy(self.title.into()),
            }]
        }

        fn ran(&mut self, action: &launcher_core::Action) {
            if let Ok(mut ran) = self.ran.lock() {
                ran.push(action.clone());
            }
        }
    }

    #[test]
    fn ran_reaches_only_the_producing_plugin() {
        let first = Arc::new(Mutex::new(Vec::new()));
        let second = Arc::new(Mutex::new(Vec::new()));
        let launcher = Launcher {
            plugins: Mutex::new(vec![
                Box::new(Recorder {
                    title: "first",
                    ran: first.clone(),
                }),
                Box::new(Recorder {
                    title: "second",
                    ran: second.clone(),
                }),
            ]),
        };
        let mut items = launcher.query("anything".into());
        assert_eq!(items.len(), 2);
        launcher.ran(items.remove(1));
        assert!(first.lock().is_ok_and(|ran| ran.is_empty()));
        assert!(
            second
                .lock()
                .is_ok_and(|ran| { *ran == [launcher_core::Action::Copy("second".into())] })
        );
    }
}
