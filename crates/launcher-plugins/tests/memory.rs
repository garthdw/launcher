//! Live heap of the plugins whose state doesn't depend on the machine, against a budget, and no
//! growth as they answer. `scripts/measure` covers the whole app. Its own test binary, so the
//! counting allocator sees only this test.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use launcher_calc::Calc;
use launcher_core::Plugins;
use launcher_jwt::Jwt;
use launcher_kagi::Kagi;

/// Raise only deliberately, saying why. Measured at 110 KB.
const BUDGET: usize = 256 * 1024;

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        LIVE.fetch_add(size, Ordering::Relaxed);
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Typed a letter at a time, then a pasted token.
fn type_script(plugins: &mut Plugins) {
    for text in [
        "100 N to lbf",
        "12 usd to eur",
        "2+2*3",
        "sqrt(2)*pi",
        "now() + 3 days",
        "0xff + 0b101",
        "3 m + 2 s",
        "visual studio code",
    ] {
        for (end, _) in text.char_indices().skip(1).chain([(text.len(), ' ')]) {
            plugins.query(&text[..end]);
        }
    }
    plugins.query(
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyLCJleHAiOjE5MTYyMzkwMjJ9.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yQsw5c",
    );
}

#[test]
fn plugins_stay_in_budget_and_dont_grow() -> Result<(), Box<dyn std::error::Error>> {
    let before = LIVE.load(Ordering::Relaxed);
    // Kagi idle, as no query starts with its prefix.
    let mut plugins = Plugins::new(vec![
        Box::new(Calc::new()),
        Box::new(Jwt),
        Box::new(Kagi::new(
            "k".into(),
            Duration::from_millis(400),
            Arc::new(|| {}),
        )),
    ]);
    // The first script fills anything built on first use.
    type_script(&mut plugins);
    let started = LIVE.load(Ordering::Relaxed) - before;
    assert!(
        started <= BUDGET,
        "{started} bytes live after starting and typing, over the budget of {BUDGET}"
    );
    for _ in 0..5 {
        type_script(&mut plugins);
    }
    let typed = LIVE.load(Ordering::Relaxed) - before;
    assert_eq!(typed, started, "live bytes grew while typing");
    Ok(())
}
