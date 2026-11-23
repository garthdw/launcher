//! The boundary the native shells call. Its types mirror `launcher-core`'s so the core stays
//! free of FFI derives; a Rust shell (Linux) uses the core directly.

use std::path::Path;
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
}

#[derive(uniffi::Enum)]
pub enum Action {
    Copy { text: String },
    Open { path: String },
}

impl From<launcher_core::Item> for Item {
    fn from(item: launcher_core::Item) -> Self {
        let action = match item.action {
            launcher_core::Action::Copy(text) => Action::Copy { text },
            launcher_core::Action::Open(path) => Action::Open {
                path: path.to_string_lossy().into_owned(),
            },
        };
        Self {
            title: item.title,
            subtitle: item.subtitle,
            action,
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
        plugins
            .iter_mut()
            .flat_map(|plugin| plugin.query(query))
            .map(Item::from)
            .collect()
    }

    /// The shell opened `path` from an `Action::Open` result.
    pub fn opened(&self, path: String) {
        let Ok(mut plugins) = self.plugins.lock() else {
            eprintln!("a plugin panicked; not recording {path}");
            return;
        };
        for plugin in plugins.iter_mut() {
            plugin.opened(Path::new(&path));
        }
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
    fn query_converts_items() {
        let launcher = Launcher::new();
        let items = launcher.query(" 2 + 3 ".into());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "5");
        assert!(matches!(&items[0].action, Action::Copy { text } if text == "5"));
        assert!(launcher.query("   ".into()).is_empty());
    }
}
