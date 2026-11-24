//! The boundary the native shells call. Its types mirror `launcher-core`'s so the core stays
//! free of FFI derives; a Rust shell (Linux) uses the core directly.

use std::sync::{Arc, Mutex};

use launcher_core::{Found, PluginId, Plugins};

uniffi::setup_scaffolding!();

#[derive(uniffi::Record)]
pub struct Item {
    pub title: String,
    pub subtitle: String,
    pub action: Action,
    pub detail: Vec<Section>,
    /// Handed back to `Launcher::ran` once the action has run.
    pub choice: Arc<Choice>,
}

#[derive(uniffi::Record)]
pub struct Section {
    pub title: String,
    pub text: String,
}

impl From<launcher_core::Section> for Section {
    fn from(section: launcher_core::Section) -> Self {
        Self {
            title: section.title,
            text: section.text,
        }
    }
}

#[derive(uniffi::Enum)]
pub enum Action {
    Copy { text: String },
    Open { path: String },
    Nothing,
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
            launcher_core::Action::Nothing => Action::Nothing,
        };
        Self {
            title: found.item.title,
            subtitle: found.item.subtitle,
            action,
            detail: found.item.detail.into_iter().map(Section::from).collect(),
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
    fn with(plugins: Plugins) -> Self {
        Self {
            plugins: Mutex::new(plugins),
        }
    }
}

/// The shell's side of `launcher_core::ResultsChanged`.
#[uniffi::export(callback_interface)]
pub trait ResultsListener: Send + Sync {
    /// Called from any thread when the results for the shown query may have changed.
    fn results_changed(&self);
}

#[uniffi::export]
impl Launcher {
    #[uniffi::constructor]
    pub fn new(listener: Box<dyn ResultsListener>) -> Arc<Self> {
        let listener: Arc<dyn ResultsListener> = Arc::from(listener);
        Arc::new(Self::with(launcher_plugins::start(Arc::new(move || {
            listener.results_changed()
        }))))
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

#[cfg(test)]
mod tests {
    use launcher_calc::Calc;
    use launcher_core::Plugin;

    use super::*;

    struct Pending;

    impl Plugin for Pending {
        fn query(&mut self, _query: &str) -> Vec<launcher_core::Item> {
            vec![launcher_core::Item {
                title: "Loading…".into(),
                subtitle: String::new(),
                action: launcher_core::Action::Nothing,
                detail: Vec::new(),
            }]
        }
    }

    #[test]
    fn query_converts_items_without_an_action() {
        let launcher = Launcher::with(Plugins::new(vec![Box::new(Pending)]));
        let items = launcher.query("anything".into());
        assert!(matches!(
            items.as_slice(),
            [Item {
                action: Action::Nothing,
                ..
            }]
        ));
    }

    #[test]
    fn query_converts_items() -> Result<(), Box<dyn std::error::Error>> {
        // Only the calculator, so installed apps can't add results.
        let launcher = Launcher::with(Plugins::new(vec![Box::new(Calc::new()?)]));
        let items = launcher.query("2 + 3".into());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "5");
        assert!(matches!(&items[0].action, Action::Copy { text } if text == "5"));
        assert_eq!(
            items[0].choice.action,
            launcher_core::Action::Copy("5".into())
        );
        assert_eq!(
            items[0]
                .detail
                .iter()
                .map(|section| (section.title.as_str(), section.text.as_str()))
                .collect::<Vec<_>>(),
            [("Input", "2 + 3")]
        );
        Ok(())
    }
}
