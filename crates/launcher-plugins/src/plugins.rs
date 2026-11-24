//! The plugins every shell runs, so a shell only draws results and runs their actions.

use launcher_apps::Apps;
use launcher_calc::Calc;
use launcher_core::{Plugin, Plugins, ResultsChanged, cache_directory};
use launcher_jwt::Jwt;

/// Builds the plugins in the order their results show, and starts their background work, which
/// calls `changed` when it may have changed the results.
pub fn start(changed: ResultsChanged) -> Plugins {
    let mut plugins: Vec<Box<dyn Plugin + Send>> = Vec::new();
    let rates_directory = cache_directory().unwrap_or_else(|| {
        eprintln!("no home folder, so exchange rates are cached in the temporary folder");
        std::env::temp_dir()
    });
    match Calc::start(rates_directory, changed.clone()) {
        Ok(calc) => plugins.push(Box::new(calc)),
        Err(error) => eprintln!("calculator unavailable: {error}"),
    }
    plugins.push(Box::new(Jwt));
    plugins.push(Box::new(Apps::start(changed)));
    Plugins::new(plugins)
}
