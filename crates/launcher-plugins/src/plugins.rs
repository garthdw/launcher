//! The plugins every shell runs, so a shell only draws results and runs their actions.

use launcher_apps::Apps;
use launcher_calc::Calc;
use launcher_core::{Plugin, Plugins, ResultsChanged, cache_directory};
use launcher_jwt::Jwt;

/// Builds the plugins in the order their results show, and starts their background work, which
/// calls `changed` when it may have changed the results.
pub fn start(changed: ResultsChanged) -> Plugins {
    let mut plugins: Vec<Box<dyn Plugin + Send>> = Vec::new();
    match Calc::new() {
        Ok(calc) => {
            match cache_directory() {
                Some(directory) => Calc::load_exchange_rates(&directory),
                None => eprintln!("no home folder to cache exchange rates in"),
            }
            plugins.push(Box::new(calc));
        }
        Err(error) => eprintln!("calculator unavailable: {error}"),
    }
    plugins.push(Box::new(Jwt));
    plugins.push(Box::new(Apps::start(changed)));
    Plugins::new(plugins)
}
