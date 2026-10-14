# Security Policy

PeekGuard is a single-process Windows tray application. It has no network code of any
kind, no dependencies, and no installer, so there is very little here to attack. This
page says what is in scope, what is deliberately out of scope, and how to report a
problem.

## Supported versions

Only the latest release is supported. There is no maintenance branch, no backporting,
and no LTS line. Upgrade to the newest release and rebuild from source if you would
rather not trust a prebuilt exe.

| Version | Supported |
|---|---|
| Latest release | Yes |
| Anything older | No |

## Reporting a vulnerability

Please use GitHub's
[private vulnerability reporting](https://github.com/Reance195/peekguard/security/advisories/new).
It keeps the report private until a fix or an advisory exists, and it is the only
channel that is actually monitored.

Do **not** open a public issue for anything that could be exploited. Reports are
triaged as time allows; this is a hobby project with no on-call rotation, so please
allow a reasonable amount of time for a first reply.

There is no security mailing list. The advisory form is the only monitored channel.

## What is in scope

- **Window restore failures.** The one unrecoverable state in this project is a window
  that stays hidden after PeekGuard exits. Anything that reaches it is a real bug.
- **Privilege or scope violations.** The application must run at medium integrity and
  must never request elevation. It must not write outside `%APPDATA%\PeekGuard` and
  must not write to the registry beyond its own `HKCU\...\Run` autostart value.
- **The autostart value.** It is written only when the tray menu item is checked, and
  it is deleted when it is unchecked. Command-line quoting and path handling there are
  in scope.
- **Config parsing.** `peekguard.conf` is read from disk and is user-writable by
  definition, so malformed or hostile input in it is in scope.

## What is out of scope

- **Cross-process window tricks.** PeekGuard deliberately never opens a handle to
  another process with write access. There is no `WriteProcessMemory`, no
  `CreateRemoteThread`, no `SetWindowsHookEx`, no subclassing. Bugs that require one of
  those do not apply.
- **Network exposure.** The program never opens a socket, never loads a URL, and never
  spawns a child process. If you find network traffic coming from `peekguard.exe`, it
  did not come from this source tree.
- **Missing code signing.** The released exe is unsigned, and signing is not currently
  in place (see the code signing policy in the README); the consequences of that —
  SmartScreen warnings and Smart App Control rejecting the file — are known and
  documented, not vulnerabilities.
- **Denial of service against yourself.** Killing the process, corrupting your own
  config, or deleting the journal while it runs is not a security boundary.

## Hardening that already exists

- No third-party code, so the dependency audit surface is empty. `Cargo.lock` contains
  nothing but this crate.
- All Win32 calls are hand-bound in `src/win.rs`, which is the complete list of system
  APIs the program can ever touch.
- Every hide is journalled to `hidden.journal` **before** the window is touched, and a
  panic hook restores everything on the next launch, including under `panic = "abort"`.
- CI runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, the test
  suite, and a build on the MSRV on every push; every published binary is built by
  that workflow with its SHA-256 checksum attached to the release.
