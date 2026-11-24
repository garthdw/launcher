//! The boundary the native shells call. Its types mirror `launcher-core`'s so the core stays
//! free of FFI derives; a Rust shell (Linux) uses the core directly.

use std::sync::{Arc, Mutex};

use launcher_apps::Apps;
use launcher_calc::Calc;
use launcher_core::{Found, Plugin, PluginId, Plugins};

uniffi::setup_scaffolding!();

#[derive(uniffi::Record)]
pub struct Item {
    pub title: String,
    pub subtitle: String,
    pub action: Action,
    /// Handed back to `Launcher::ran` once the action has run.
    pub choice: Arc<Choice>,
}

#[derive(uniffi::Enum)]
pub enum Action {
    Copy { text: String },
    Open { path: String },
}

/// Which plugin an item came from, and its action as the plugin made it: paths stay `PathBuf`s
/// rather than the lossy strings the shell sees.
#[derive(uniffi::Object)]
pub struct Choice {
    plugin: PluginId,
    action: launcher_core::Action,
}

impl From<Found> for Item {
    fn from(found: Found) -> Self {
        let action = match &found.item.action {
            launcher_core::Action::Copy(text) => Action::Copy { text: text.clone() },
            launcher_core::Action::Open(path) => Action::Open {
                path: path.to_string_lossy().into_owned(),
            },
        };
        Self {
            title: found.item.title,
            subtitle: found.item.subtitle,
            action,
            choice: Arc::new(Choice {
                plugin: found.plugin,
                action: found.item.action,
            }),
        }
    }
}

#[derive(uniffi::Object)]
pub struct Launcher {
    plugins: Mutex<Plugins>,
}

impl Launcher {
    fn with(plugins: Vec<Box<dyn Plugin + Send>>) -> Self {
        Self {
            plugins: Mutex::new(Plugins::new(plugins)),
        }
    }
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
        Arc::new(Self::with(plugins))
    }

    /// Called on every keystroke from the UI thread, so plugins must answer well inside a frame.
    pub fn query(&self, query: String) -> Vec<Item> {
        let Ok(mut plugins) = self.plugins.lock() else {
            eprintln!("a plugin panicked; skipping queries");
            return Vec::new();
        };
        plugins.query(&query).into_iter().map(Item::from).collect()
    }

    /// The shell ran an item's action successfully.
    pub fn ran(&self, choice: Arc<Choice>) {
        let Ok(mut plugins) = self.plugins.lock() else {
            eprintln!("a plugin panicked; not telling it {:?} ran", choice.action);
            return;
        };
        plugins.ran(choice.plugin, &choice.action);
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
        let launcher = Launcher::with(vec![Box::new(Calc::new()?)]);
        let items = launcher.query("2 + 3".into());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "5");
        assert!(matches!(&items[0].action, Action::Copy { text } if text == "5"));
        assert_eq!(
            items[0].choice.action,
            launcher_core::Action::Copy("5".into())
        );
        Ok(())
    }
}
