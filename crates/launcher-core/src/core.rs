use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub title: String,
    pub subtitle: String,
    pub action: Action,
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
