//! Opens the system terminal in the current folder.
//! GIO does not expose terminals: the executable is looked up in PATH
//! and the known `--working-directory` / `-e` options are used. The root
//! shell runs inside the terminal (`sudo -s`): `pkexec` would sanitize
//! the environment and the terminal would not reach the Wayland display.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;

use crate::preferences::model::TerminalChoice;

#[derive(Debug)]
pub enum TerminalError {
    NoTerminal,
    NoRootShell,
    UnsupportedSelected(String),
    SelectedWithoutRoot(String),
    Launch(String),
}

impl TerminalError {
    /// A localized explanation; launch failures retain the technical GIO/OS detail.
    pub fn user_message(&self) -> String {
        match self {
            Self::NoTerminal => crate::l10n::tr("term-no-term"),
            Self::NoRootShell => crate::l10n::tr("term-no-root"),
            Self::UnsupportedSelected(terminal) => {
                crate::l10n::tr_with_one("term-selected-unsupported", "terminal", terminal)
            }
            Self::SelectedWithoutRoot(terminal) => {
                crate::l10n::tr_with_one("term-selected-no-root", "terminal", terminal)
            }
            Self::Launch(error) => crate::l10n::tr_with_one("term-launch-error", "error", error),
        }
    }
}

/// A known terminal emulator: how to open it in a folder and how to
/// open a root shell in it. Arguments stay `OsString` from the `Path`
/// down to the command: no lossy conversion, non-UTF-8 folders work.
/// An empty `cwd` list is valid (the workdir is set on the command);
/// `root: None` means a root shell is unsupported and the terminal is
/// skipped when one is requested.
struct Terminal {
    prog: &'static str,
    /// Arguments to open the terminal in `dir`.
    cwd: fn(&Path) -> Vec<OsString>,
    /// Arguments for a root shell in `dir`; `None` = unsupported.
    root: Option<fn(&Path) -> Vec<OsString>>,
}

/// `key=value` as a single `OsString`, preserving `dir` bytes.
fn joined(key: &str, dir: &Path) -> OsString {
    let mut arg = OsString::from(key);
    arg.push(dir);
    arg
}

const TERMINALS: [Terminal; 11] = [
    Terminal {
        prog: "konsole",
        cwd: |d| vec![OsString::from("--workdir"), d.as_os_str().to_owned()],
        root: Some(|d| {
            vec![
                OsString::from("--workdir"),
                d.as_os_str().to_owned(),
                OsString::from("-e"),
                OsString::from("sudo"),
                OsString::from("-s"),
            ]
        }),
    },
    Terminal {
        prog: "gnome-terminal",
        cwd: |d| vec![joined("--working-directory=", d)],
        root: Some(|d| {
            vec![
                joined("--working-directory=", d),
                OsString::from("--"),
                OsString::from("sudo"),
                OsString::from("-s"),
            ]
        }),
    },
    Terminal {
        prog: "ptyxis",
        cwd: |d| {
            vec![
                OsString::from("--new-window"),
                joined("--working-directory=", d),
            ]
        },
        root: Some(|d| {
            vec![
                OsString::from("--new-window"),
                joined("--working-directory=", d),
                OsString::from("--"),
                OsString::from("sudo"),
                OsString::from("-s"),
            ]
        }),
    },
    Terminal {
        prog: "kgx",
        cwd: |d| {
            vec![
                OsString::from("--working-directory"),
                d.as_os_str().to_owned(),
            ]
        },
        root: Some(|d| {
            vec![
                OsString::from("--working-directory"),
                d.as_os_str().to_owned(),
                OsString::from("--"),
                OsString::from("sudo"),
                OsString::from("-s"),
            ]
        }),
    },
    Terminal {
        prog: "xfce4-terminal",
        cwd: |d| vec![joined("--working-directory=", d)],
        root: Some(|d| {
            vec![
                joined("--working-directory=", d),
                OsString::from("-x"),
                OsString::from("sudo"),
                OsString::from("-s"),
            ]
        }),
    },
    Terminal {
        prog: "tilix",
        cwd: |d| vec![joined("--working-directory=", d)],
        root: Some(|d| {
            vec![
                joined("--working-directory=", d),
                OsString::from("-e"),
                OsString::from("sudo -s"),
            ]
        }),
    },
    Terminal {
        prog: "alacritty",
        cwd: |_| vec![],
        root: Some(|_| {
            vec![
                OsString::from("-e"),
                OsString::from("sudo"),
                OsString::from("-s"),
            ]
        }),
    },
    Terminal {
        prog: "kitty",
        cwd: |_| vec![],
        root: Some(|_| vec![OsString::from("sudo"), OsString::from("-s")]),
    },
    Terminal {
        prog: "wezterm",
        cwd: |d| {
            vec![
                OsString::from("start"),
                OsString::from("--cwd"),
                d.as_os_str().to_owned(),
            ]
        },
        root: Some(|d| {
            vec![
                OsString::from("start"),
                OsString::from("--cwd"),
                d.as_os_str().to_owned(),
                OsString::from("--"),
                OsString::from("sudo"),
                OsString::from("-s"),
            ]
        }),
    },
    Terminal {
        prog: "foot",
        cwd: |d| vec![joined("--working-dir=", d)],
        root: None,
    },
    Terminal {
        prog: "xterm",
        cwd: |_| vec![],
        root: Some(|_| {
            vec![
                OsString::from("-e"),
                OsString::from("sudo"),
                OsString::from("-s"),
            ]
        }),
    },
];

/// `true` if `prog` is an executable in PATH.
fn which(prog: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    which_in_path(prog, &path)
}

/// Inspect the actual process PATH without requiring its directories to be UTF-8.
fn which_in_path(prog: &str, path: &OsStr) -> bool {
    std::env::split_paths(path).any(|dir| {
        let candidate = dir.join(prog);
        use std::os::unix::fs::PermissionsExt as _;
        candidate
            .metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

/// Returns only known terminal emulators that are currently executable on
/// PATH. Detection inspects filesystem metadata only; it never launches one.
pub fn available_terminal_programs() -> Vec<String> {
    TERMINALS
        .iter()
        .filter(|terminal| which(terminal.prog))
        .map(|terminal| terminal.prog.to_string())
        .collect()
}

pub fn display_name(program: &str) -> String {
    let pretty = match program {
        "konsole" => "Konsole",
        "gnome-terminal" => "GNOME Terminal",
        "ptyxis" => "Ptyxis",
        "kgx" => "GNOME Console",
        "xfce4-terminal" => "Xfce Terminal",
        "tilix" => "Tilix",
        "alacritty" => "Alacritty",
        "kitty" => "Kitty",
        "wezterm" => "WezTerm",
        "foot" => "Foot",
        "xterm" => "XTerm",
        other => return other.to_string(),
    };
    format!("{pretty} ({program})")
}

/// Whether `open(dir, root, choice)` could succeed with `available`
/// programs installed, without spawning anything. Mirrors the selection
/// logic for menu enablement: a missing preferred emulator falls back to
/// automatic detection, an unknown one never works.
pub fn can_open(root: bool, choice: &TerminalChoice, available: &[String]) -> bool {
    fn known_with_root(program: &str, root: bool) -> bool {
        TERMINALS
            .iter()
            .find(|terminal| terminal.prog == program)
            .is_some_and(|terminal| !root || terminal.root.is_some())
    }
    match choice {
        TerminalChoice::Automatic => {
            if root {
                TERMINALS
                    .iter()
                    .any(|t| t.root.is_some() && available.iter().any(|a| a == t.prog))
            } else {
                !available.is_empty()
            }
        }
        TerminalChoice::Emulator(program) => {
            if available.iter().any(|name| name == program) {
                known_with_root(program, root)
            } else if root {
                TERMINALS
                    .iter()
                    .any(|t| t.root.is_some() && available.iter().any(|a| a == t.prog))
            } else {
                !available.is_empty()
            }
        }
    }
}
/// Picks the first available terminal and builds `(program, args)`,
/// without spawning anything. Testable with a fake table.
/// A normal launch may validly have zero arguments (the workdir is set
/// on the command); a requested root shell skips terminals without root
/// support and uses the next compatible one.
fn select_command<'t>(
    dir: &Path,
    root: bool,
    terms: &'t [Terminal],
    mut available: impl FnMut(&str) -> bool,
) -> Result<(&'t str, Vec<OsString>), TerminalError> {
    let mut any_available = false;
    for terminal in terms {
        if !available(terminal.prog) {
            continue;
        }
        any_available = true;
        if root {
            let Some(build) = terminal.root else {
                continue;
            };
            return Ok((terminal.prog, build(dir)));
        }
        return Ok((terminal.prog, (terminal.cwd)(dir)));
    }
    if root && any_available {
        return Err(TerminalError::NoRootShell);
    }
    Err(TerminalError::NoTerminal)
}

/// Uses an explicitly selected available emulator, or falls back to the
/// automatic search if that saved emulator has disappeared since startup.
fn select_preferred_command<'t>(
    dir: &Path,
    root: bool,
    choice: &TerminalChoice,
    terms: &'t [Terminal],
    mut available: impl FnMut(&str) -> bool,
) -> Result<(&'t str, Vec<OsString>), TerminalError> {
    let TerminalChoice::Emulator(program) = choice else {
        return select_command(dir, root, terms, available);
    };
    if !available(program) {
        return select_command(dir, root, terms, available);
    }
    let Some(terminal) = terms.iter().find(|terminal| terminal.prog == program) else {
        return Err(TerminalError::UnsupportedSelected(program.clone()));
    };
    let args = if root {
        let Some(build) = terminal.root else {
            return Err(TerminalError::SelectedWithoutRoot(display_name(program)));
        };
        build(dir)
    } else {
        (terminal.cwd)(dir)
    };
    Ok((terminal.prog, args))
}

/// Spawns `prog` with `args` in `dir`. Split out so argument building
/// is verifiable without spawning real terminals.
fn spawn_command(prog: &str, args: &[OsString], dir: &Path) -> Result<(), TerminalError> {
    Command::new(prog)
        .args(args)
        .current_dir(dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| TerminalError::Launch(e.to_string()))
}

/// Opens the terminal in `dir`. With `root` the shell already starts
/// as root (`sudo -s` inside the terminal: `sudo -i` would `cd` into
/// root's home, `-s` stays in the current folder instead).
pub fn open(dir: &Path, root: bool, choice: &TerminalChoice) -> Result<(), TerminalError> {
    let (prog, args) = select_preferred_command(dir, root, choice, &TERMINALS, which)?;
    spawn_command(prog, &args, dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    const FAKE_TERMS: [Terminal; 3] = [
        Terminal {
            prog: "missing-term",
            cwd: |d| vec![joined("--dir=", d)],
            root: None,
        },
        Terminal {
            prog: "fake-term",
            cwd: |d| vec![joined("--dir=", d)],
            root: Some(|d| {
                vec![
                    joined("--dir=", d),
                    OsString::from("-e"),
                    OsString::from("sudo -s"),
                ]
            }),
        },
        Terminal {
            prog: "other-term",
            cwd: |d| vec![d.as_os_str().to_owned()],
            root: Some(|d| vec![d.as_os_str().to_owned()]),
        },
    ];

    #[test]
    fn select_skips_unavailable_and_builds_args() {
        let dir = Path::new("/tmp/My Folder");
        let (prog, args) = select_command(dir, false, &FAKE_TERMS, |p| p == "fake-term").unwrap();
        assert_eq!(prog, "fake-term");
        assert_eq!(args, vec![OsString::from("--dir=/tmp/My Folder")]);
    }

    #[test]
    fn normal_launch_allows_zero_args() {
        // Alacritty, Kitty and Xterm run with no arguments; the workdir
        // comes from the command itself. Real table, no PATH lookup.
        let dir = Path::new("/tmp/My Folder");
        for prog in ["alacritty", "kitty", "xterm"] {
            let (picked, args) = select_command(dir, false, &TERMINALS, |p| p == prog).unwrap();
            assert_eq!(picked, prog);
            assert!(args.is_empty());
        }
    }

    #[test]
    fn normal_launch_with_args() {
        let dir = Path::new("/tmp/My Folder");
        let (prog, args) = select_command(dir, false, &TERMINALS, |p| p == "konsole").unwrap();
        assert_eq!(prog, "konsole");
        assert_eq!(
            args,
            vec![
                OsString::from("--workdir"),
                OsString::from("/tmp/My Folder")
            ]
        );
    }

    #[test]
    fn root_command_for_compatible_terminal() {
        let dir = Path::new("/tmp/My Folder");
        let (prog, args) =
            select_command(dir, true, &TERMINALS, |p| p == "gnome-terminal").unwrap();
        assert_eq!(prog, "gnome-terminal");
        assert_eq!(
            args,
            vec![
                OsString::from("--working-directory=/tmp/My Folder"),
                OsString::from("--"),
                OsString::from("sudo"),
                OsString::from("-s"),
            ]
        );
    }

    #[test]
    fn ptyxis_is_selectable_for_normal_and_root_launches() {
        let dir = Path::new(OsStr::from_bytes(b"/tmp/My \xff Folder"));
        let choice = TerminalChoice::Emulator("ptyxis".to_string());
        let available = available_of(&["ptyxis"]);
        for root in [false, true] {
            let (prog, args) = select_command(dir, root, &TERMINALS, |p| p == "ptyxis").unwrap();
            assert_eq!(prog, "ptyxis");
            let mut expected = vec![
                OsString::from("--new-window"),
                joined("--working-directory=", dir),
            ];
            if root {
                expected.extend(["--", "sudo", "-s"].map(OsString::from));
            }
            assert_eq!(args, expected);
            let (preferred, preferred_args) =
                select_preferred_command(dir, root, &choice, &TERMINALS, |p| p == "ptyxis")
                    .unwrap();
            assert_eq!(preferred, prog);
            assert_eq!(preferred_args, args);
            assert!(can_open(root, &TerminalChoice::Automatic, &available));
            assert!(can_open(root, &choice, &available));
        }
        assert_eq!(display_name("ptyxis"), "Ptyxis (ptyxis)");
    }

    #[test]
    fn path_detection_preserves_non_utf8_directories() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join(OsStr::from_bytes(b"bin-\xff"));
        std::fs::create_dir(&bin).unwrap();
        let program = bin.join("ptyxis");
        std::fs::write(&program, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::join_paths([tmp.path().join("missing"), bin]).unwrap();
        assert!(which_in_path("ptyxis", &path));
        assert!(!which_in_path("missing-terminal", &path));
    }

    #[test]
    fn path_detection_requires_executable_files_and_follows_symlinks() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let tmp = tempfile::tempdir().unwrap();
        let program = tmp.path().join("ptyxis");
        std::fs::write(&program, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!which_in_path("ptyxis", tmp.path().as_os_str()));
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(which_in_path("ptyxis", tmp.path().as_os_str()));
        symlink(&program, tmp.path().join("terminal-link")).unwrap();
        assert!(which_in_path("terminal-link", tmp.path().as_os_str()));
        std::fs::create_dir(tmp.path().join("terminal-directory")).unwrap();
        assert!(!which_in_path("terminal-directory", tmp.path().as_os_str()));
    }

    #[test]
    fn root_skips_unsupported_terminal() {
        // foot has no root support: the next compatible one wins.
        let dir = Path::new("/tmp/x");
        let (prog, args) =
            select_command(dir, true, &TERMINALS, |p| p == "foot" || p == "xterm").unwrap();
        assert_eq!(prog, "xterm");
        assert_eq!(
            args,
            vec![
                OsString::from("-e"),
                OsString::from("sudo"),
                OsString::from("-s")
            ]
        );
        // Fake table, same shape.
        let (prog, _) = select_command(dir, true, &FAKE_TERMS, |_| true).unwrap();
        assert_eq!(prog, "fake-term");
    }

    #[test]
    fn no_compatible_terminal_errors() {
        let dir = Path::new("/tmp/x");
        let err = select_command(dir, true, &TERMINALS, |p| p == "foot").unwrap_err();
        assert!(matches!(err, TerminalError::NoRootShell));
        let err = select_command(dir, false, &TERMINALS, |_| false).unwrap_err();
        assert!(matches!(err, TerminalError::NoTerminal));
        let err = select_command(dir, true, &TERMINALS, |_| false).unwrap_err();
        assert!(matches!(err, TerminalError::NoTerminal));
    }

    #[test]
    fn select_preserves_non_utf8_bytes() {
        let raw = b"/tmp/bad \xff dir";
        let dir = Path::new(OsStr::from_bytes(raw));
        let (prog, args) = select_command(dir, false, &FAKE_TERMS, |p| p == "other-term").unwrap();
        assert_eq!(prog, "other-term");
        assert_eq!(args, vec![OsStr::from_bytes(raw).to_owned()]);
    }

    #[test]
    fn saved_terminal_that_disappeared_falls_back_to_automatic() {
        let dir = Path::new("/tmp/folder");
        let choice = TerminalChoice::Emulator("missing-term".to_string());
        let (prog, args) = select_preferred_command(dir, false, &choice, &FAKE_TERMS, |name| {
            name == "other-term"
        })
        .unwrap();
        assert_eq!(prog, "other-term");
        assert_eq!(args, vec![dir.as_os_str().to_owned()]);
    }

    #[test]
    fn manual_terminal_is_used_and_root_support_is_checked() {
        let dir = Path::new("/tmp/folder");
        let selected = TerminalChoice::Emulator("foot".to_string());
        let (prog, _) =
            select_preferred_command(dir, false, &selected, &TERMINALS, |name| name == "foot")
                .unwrap();
        assert_eq!(prog, "foot");

        let error =
            select_preferred_command(dir, true, &selected, &TERMINALS, |name| name == "foot")
                .unwrap_err();
        assert!(matches!(error, TerminalError::SelectedWithoutRoot(_)));
    }

    #[test]
    fn spawn_uses_workdir_without_real_terminal() {
        // `/bin/true` exits immediately: proves program + workdir wiring.
        let tmp = std::env::temp_dir();
        assert!(tmp.is_dir());
        assert!(spawn_command("/bin/true", &[], &tmp).is_ok());
        assert!(matches!(
            spawn_command("/bin/does-not-exist-kito", &[], &tmp),
            Err(TerminalError::Launch(_))
        ));
    }

    fn available_of(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn can_open_matches_selection_logic() {
        let auto = TerminalChoice::Automatic;
        assert!(!can_open(false, &auto, &[]));
        assert!(!can_open(true, &auto, &[]));
        // Only foot (no root support) installed.
        let foot = available_of(&["foot"]);
        assert!(can_open(false, &auto, &foot));
        assert!(!can_open(true, &auto, &foot));
        // Kitty supports root.
        let kitty = available_of(&["kitty"]);
        assert!(can_open(false, &auto, &kitty));
        assert!(can_open(true, &auto, &kitty));

        // Preferred emulator present and known.
        let pref_foot = TerminalChoice::Emulator("foot".to_string());
        assert!(can_open(false, &pref_foot, &foot));
        assert!(!can_open(true, &pref_foot, &foot));
        // Preferred emulator unknown to the table: never works directly.
        let pref_odd = TerminalChoice::Emulator("odd-term".to_string());
        assert!(!can_open(false, &pref_odd, &available_of(&["odd-term"])));
        // Missing preferred emulator falls back to automatic detection.
        assert!(can_open(false, &pref_foot, &kitty));
        assert!(can_open(true, &pref_foot, &kitty));
        assert!(!can_open(false, &pref_foot, &[]));
    }
}
