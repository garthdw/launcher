import AppKit

/// Sends the log to `~/Library/Logs/launcher.log` when stderr is `/dev/null`, as it is for an app
/// opened by Finder, `open` or at login, then notes the launch there. Moving file descriptor 2
/// catches the Rust core's `eprintln!` as well as `log`. A terminal, pipe or file on stderr is
/// left alone: whoever started the app is reading it there.
func startLog() {
    if stderrIsDiscarded() {
        let path = FileManager.default.homeDirectoryForCurrentUser
            .appending(path: "Library/Logs/launcher.log").path
        // One older file keeps the log bounded without a logging framework.
        var rotateFailure: String?
        if let size = try? FileManager.default.attributesOfItem(atPath: path)[.size] as? Int,
           size > 1_000_000, rename(path, path + ".1") != 0 {
            rotateFailure = "can't rotate \(path): \(String(cString: strerror(errno)))"
        }
        let file = open(path, O_WRONLY | O_APPEND | O_CREAT, 0o644)
        // On failure there's nowhere to say so: stderr still goes nowhere.
        guard file >= 0 else { return }
        // With stderr closed, `open` returns descriptor 2 itself.
        if file != STDERR_FILENO {
            dup2(file, STDERR_FILENO)
            close(file)
        }
        if let rotateFailure { log(rotateFailure) }
    }
    let trusted = AXIsProcessTrusted() ? "granted" : "missing"
    log("launched \(Date.now.ISO8601Format()) from \(Bundle.main.bundlePath); Accessibility permission \(trusted)")
}

private func stderrIsDiscarded() -> Bool {
    var stderrStatus = stat()
    var null = stat()
    guard fstat(STDERR_FILENO, &stderrStatus) == 0 else { return true }
    return stat("/dev/null", &null) == 0
        && stderrStatus.st_mode & S_IFMT == S_IFCHR && stderrStatus.st_rdev == null.st_rdev
}
