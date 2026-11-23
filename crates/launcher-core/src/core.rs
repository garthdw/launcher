use std::path::{Path, PathBuf};

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
    std::env::var_os("HOME").map(|home| Path::new(&home).join(".config/launcher"))
}

pub trait Plugin {
    /// Runs on the UI thread for every keystroke, so it must stay well under a frame.
    fn query(&mut self, query: &str) -> Vec<Item>;

    /// The shell opened `path` from some plugin's `Action::Open`.
    fn opened(&mut self, _path: &Path) {}
}
