#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub title: String,
    pub subtitle: String,
    pub action: Action,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Copy(String),
}

pub trait Plugin {
    /// Runs on the UI thread for every keystroke, so it must stay well under a frame.
    fn query(&mut self, query: &str) -> Vec<Item>;
}
