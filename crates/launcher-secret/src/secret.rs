//! Secrets such as API tokens, kept in the platform's secret store rather than the config file:
//! the login keychain on macOS, as generic passwords of the app's bundle identifier.
//! Plugins read them when they need them, so none stays in memory.

use std::io::{self, BufRead, IsTerminal, Write};
use std::sync::OnceLock;

/// The secret stored as `name`, or `None` without one.
pub use store::read;

/// The command line that sets the secret `name`, naming this binary, quoted for a shell.
pub fn set_command(name: &str) -> String {
    let launcher = std::env::current_exe()
        .map(|path| format!("'{}'", path.to_string_lossy().replace('\'', r"'\''")))
        .unwrap_or_else(|_| "Launcher".into());
    format!("{launcher} secret set {name}")
}

/// Runs `secret` commands, which every shell shares: `secret set NAME` reads the secret from
/// stdin, so it never lands in shell history, and `secret remove NAME`. `names` are the secrets
/// plugins read. `None` when the arguments aren't a command, so the UI starts; else the exit
/// status.
#[expect(
    clippy::disallowed_macros,
    reason = "a command's messages go to its terminal, not the log"
)]
pub fn run_command(arguments: &[String], names: &[&str]) -> Option<i32> {
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let ["secret", command @ ..] = arguments.as_slice() else {
        return None;
    };
    let (name, result) = match command {
        ["set", name] if names.contains(name) => (
            *name,
            read_stdin(name).and_then(|secret| store::set(name, &secret).map(|()| "stored")),
        ),
        ["remove", name] if names.contains(name) => (
            *name,
            store::remove(name).map(|removed| if removed { "removed" } else { "not stored" }),
        ),
        _ => {
            eprintln!(
                "usage: secret set NAME | secret remove NAME\n  NAME is one of {}; set reads \
                 the secret from stdin, as in `pbpaste | Launcher secret set kagi`",
                names.join(", ")
            );
            return Some(64);
        }
    };
    Some(match result {
        Ok(done) => {
            println!("{name}: {done}");
            0
        }
        Err(error) => {
            eprintln!("{name}: {error}");
            1
        }
    })
}

/// The first line of stdin, prompting with echo off when it's a terminal, so the secret isn't
/// left on screen.
#[expect(
    clippy::disallowed_macros,
    reason = "the prompt goes to the terminal, not the log"
)]
fn read_stdin(name: &str) -> Result<String, String> {
    let stdin = io::stdin();
    let echo = if stdin.is_terminal() {
        eprint!("{name}: ");
        io::stderr()
            .flush()
            .map_err(|error| format!("can't prompt: {error}"))?;
        Echo::off()
    } else {
        None
    };
    let mut line = String::new();
    let read = stdin.lock().read_line(&mut line);
    if echo.is_some() {
        drop(echo);
        eprintln!();
    }
    read.map_err(|error| format!("can't read stdin: {error}"))?;
    let secret = line.trim();
    if secret.is_empty() {
        return Err("nothing on stdin, so nothing stored".into());
    }
    Ok(secret.to_owned())
}

/// The terminal's settings before echo went off, put back on drop, or by `interrupted` should
/// Ctrl-C or a kill end the process first.
static SAVED: OnceLock<libc::termios> = OnceLock::new();

/// Echo turned off on the terminal on stdin while it lives.
struct Echo;

const SIGNALS: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

impl Echo {
    /// `None` when stdin's settings can't be changed, so echo stays on.
    fn off() -> Option<Self> {
        // SAFETY: `termios` is plain data that `tcgetattr` fills on success; `interrupted` only
        // makes calls that are safe in a signal handler.
        unsafe {
            let mut settings: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut settings) != 0 {
                return None;
            }
            let saved = *SAVED.get_or_init(|| settings);
            for signal in SIGNALS {
                libc::signal(signal, interrupted as *const () as libc::sighandler_t);
            }
            settings.c_lflag &= !libc::ECHO;
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &settings) != 0 {
                restore(&saved);
                return None;
            }
        }
        Some(Self)
    }
}

impl Drop for Echo {
    fn drop(&mut self) {
        if let Some(saved) = SAVED.get() {
            // SAFETY: as in `off`.
            unsafe { restore(saved) };
        }
    }
}

/// Puts back the terminal's settings and the signals' default handling.
///
/// # Safety
///
/// Only calls functions that are safe in a signal handler.
unsafe fn restore(saved: &libc::termios) {
    unsafe {
        libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, saved);
        for signal in SIGNALS {
            libc::signal(signal, libc::SIG_DFL);
        }
    }
}

/// Puts the terminal back, then lets the signal end the process as it would have.
extern "C" fn interrupted(signal: libc::c_int) {
    if let Some(saved) = SAVED.get() {
        // SAFETY: `restore` and `raise` are safe in a signal handler.
        unsafe {
            restore(saved);
            libc::raise(signal);
        }
    }
}

#[cfg(target_os = "macos")]
mod store {
    use std::ptr;

    use launcher_core::BUNDLE_IDENTIFIER;
    use objc2_core_foundation::{CFBoolean, CFData, CFDictionary, CFRetained, CFString, CFType};
    use objc2_security::{
        SecCopyErrorMessageString, SecItemAdd, SecItemCopyMatching, SecItemDelete, SecItemUpdate,
        errSecDuplicateItem, errSecItemNotFound, errSecSuccess, kSecAttrAccount, kSecAttrService,
        kSecClass, kSecClassGenericPassword, kSecMatchLimit, kSecMatchLimitOne, kSecReturnData,
        kSecValueData,
    };

    /// A dictionary naming `name`'s item, with `more` keys and values.
    fn item(
        name: &str,
        more: &[(&CFString, &CFType)],
    ) -> CFRetained<CFDictionary<CFString, CFType>> {
        let service = CFString::from_str(BUNDLE_IDENTIFIER);
        let account = CFString::from_str(name);
        // SAFETY: Security's constants, valid for the life of the process.
        let (class, generic, service_key, account_key) = unsafe {
            (
                kSecClass,
                kSecClassGenericPassword,
                kSecAttrService,
                kSecAttrAccount,
            )
        };
        let keys: Vec<&CFString> = [class, service_key, account_key]
            .into_iter()
            .chain(more.iter().map(|(key, _)| *key))
            .collect();
        let values: Vec<&CFType> = [generic.as_ref(), service.as_ref(), account.as_ref()]
            .into_iter()
            .chain(more.iter().map(|(_, value)| *value))
            .collect();
        CFDictionary::from_slices(&keys, &values)
    }

    fn failure(doing: &str, status: i32) -> String {
        // SAFETY: a null `reserved`, as documented.
        let message = unsafe { SecCopyErrorMessageString(status, ptr::null_mut()) };
        match message {
            Some(message) => format!("can't {doing} the keychain: {message}"),
            None => format!("can't {doing} the keychain: error {status}"),
        }
    }

    pub fn read(name: &str) -> Result<Option<String>, String> {
        // SAFETY: Security's constants, valid for the life of the process.
        let (return_data, match_limit, one) =
            unsafe { (kSecReturnData, kSecMatchLimit, kSecMatchLimitOne) };
        let query = item(
            name,
            &[
                (return_data, CFBoolean::new(true).as_ref()),
                (match_limit, one.as_ref()),
            ],
        );
        let mut found: *const CFType = ptr::null();
        // SAFETY: `found` receives a +1 retained CFData on success, owned by `CFRetained` below.
        let status = unsafe { SecItemCopyMatching(query.as_opaque(), &mut found) };
        if status == errSecItemNotFound {
            return Ok(None);
        }
        if status != errSecSuccess {
            return Err(failure("read from", status));
        }
        let found = ptr::NonNull::new(found.cast_mut())
            .ok_or_else(|| "the keychain returned nothing".to_owned())?;
        // SAFETY: see above.
        let found = unsafe { CFRetained::from_raw(found) };
        let data = found
            .downcast::<CFData>()
            .map_err(|_| "the keychain returned something other than data".to_owned())?;
        String::from_utf8(data.to_vec())
            .map(Some)
            .map_err(|_| format!("the keychain's {name} isn't UTF-8"))
    }

    pub(super) fn set(name: &str, secret: &str) -> Result<(), String> {
        let data = CFData::from_bytes(secret.as_bytes());
        // SAFETY: Security's constant, valid for the life of the process.
        let value_key = unsafe { kSecValueData };
        let value: (&CFString, &CFType) = (value_key, data.as_ref());
        // SAFETY: the dictionaries are well formed, and no result is asked for.
        let status = unsafe { SecItemAdd(item(name, &[value]).as_opaque(), ptr::null_mut()) };
        let status = if status == errSecDuplicateItem {
            let update = CFDictionary::<CFString, CFType>::from_slices(&[value.0], &[value.1]);
            // SAFETY: as above.
            unsafe { SecItemUpdate(item(name, &[]).as_opaque(), update.as_opaque()) }
        } else {
            status
        };
        if status == errSecSuccess {
            Ok(())
        } else {
            Err(failure("write to", status))
        }
    }

    pub(super) fn remove(name: &str) -> Result<bool, String> {
        // SAFETY: the dictionary is well formed.
        match unsafe { SecItemDelete(item(name, &[]).as_opaque()) } {
            status if status == errSecSuccess => Ok(true),
            status if status == errSecItemNotFound => Ok(false),
            status => Err(failure("remove from", status)),
        }
    }
}

/// The Secret Service over D-Bus will serve Linux.
#[cfg(not(target_os = "macos"))]
mod store {
    const UNSUPPORTED: &str = "no secret store on this platform yet";

    pub fn read(_name: &str) -> Result<Option<String>, String> {
        Err(UNSUPPORTED.into())
    }

    pub(super) fn set(_name: &str, _secret: &str) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    pub(super) fn remove(_name: &str) -> Result<bool, String> {
        Err(UNSUPPORTED.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_arguments_start_the_app() {
        let names = ["kagi"];
        let run = |arguments: &[&str]| {
            let arguments: Vec<String> =
                arguments.iter().map(|&argument| argument.into()).collect();
            run_command(&arguments, &names)
        };
        assert_eq!(run(&[]), None);
        assert_eq!(run(&["config", "init"]), None);
        assert_eq!(run(&["secret"]), Some(64));
        assert_eq!(run(&["secret", "set"]), Some(64));
        assert_eq!(run(&["secret", "set", "kagj"]), Some(64), "an unknown name");
        assert_eq!(run(&["secret", "show", "kagi"]), Some(64));
        assert_eq!(
            run(&["secret", "set", "kagi", "token"]),
            Some(64),
            "no secret in arguments"
        );
    }
}
