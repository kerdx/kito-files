//! Opens the system terminal in the current folder.
//! GIO does not expose terminals: the executable is looked up in PATH
//! and the known `--working-directory` / `-e` options are used. The root
//! shell runs inside the terminal (`sudo -s`): `pkexec` would sanitize
//! the environment and the terminal would not reach the Wayland display.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

/// A known terminal emulator: how to open it in a folder and how to
/// open a root shell in it. Arguments stay `OsString` from the `Path`
/// down to the command: no lossy conversion, non-UTF-8 folders work.
struct Terminal {
    prog: &'static str,
    /// Arguments to open the terminal in `dir`.
    cwd: fn(&Path) -> Vec<OsString>,
    /// Arguments for a root shell in `dir`; empty = unsupported.
    root: fn(&Path) -> Vec<OsString>,
}

/// `key=value` as a single `OsString`, preserving `dir` bytes.
fn joined(key: &str, dir: &Path) -> OsString {
    let mut arg = OsString::from(key);
    arg.push(dir);
    arg
}

const TERMINALS: [Terminal; 10] = [
    Terminal {
        prog: "konsole",
        cwd: |d| vec![OsString::from("--workdir"), d.as_os_str().to_owned()],
        root: |d| {
            vec![
                OsString::from("--workdir"),
                d.as_os_str().to_owned(),
                OsString::from("-e"),
                OsString::from("sudo"),
                OsString::from("-s"),
            ]
        },
    },
    Terminal {
        prog: "gnome-terminal",
        cwd: |d| vec![joined("--working-directory=", d)],
        root: |d| {
            vec![
                joined("--working-directory=", d),
                OsString::from("--"),
                OsString::from("sudo"),
                OsString::from("-s"),
            ]
        },
    },
    Terminal {
        prog: "kgx",
        cwd: |d| {
            vec![
                OsString::from("--working-directory"),
                d.as_os_str().to_owned(),
            ]
        },
        root: |d| {
            vec![
                OsString::from("--working-directory"),
                d.as_os_str().to_owned(),
                OsString::from("--"),
                OsString::from("sudo"),
                OsString::from("-s"),
            ]
        },
    },
    Terminal {
        prog: "xfce4-terminal",
        cwd: |d| vec![joined("--working-directory=", d)],
        root: |d| {
            vec![
                joined("--working-directory=", d),
                OsString::from("-x"),
                OsString::from("sudo"),
                OsString::from("-s"),
            ]
        },
    },
    Terminal {
        prog: "tilix",
        cwd: |d| vec![joined("--working-directory=", d)],
        root: |d| {
            vec![
                joined("--working-directory=", d),
                OsString::from("-e"),
                OsString::from("sudo -s"),
            ]
        },
    },
    Terminal {
        prog: "alacritty",
        cwd: |_| vec![],
        root: |_| {
            vec![
                OsString::from("-e"),
                OsString::from("sudo"),
                OsString::from("-s"),
            ]
        },
    },
    Terminal {
        prog: "kitty",
        cwd: |_| vec![],
        root: |_| vec![OsString::from("sudo"), OsString::from("-s")],
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
        root: |d| {
            vec![
                OsString::from("start"),
                OsString::from("--cwd"),
                d.as_os_str().to_owned(),
                OsString::from("--"),
                OsString::from("sudo"),
                OsString::from("-s"),
            ]
        },
    },
    Terminal {
        prog: "foot",
        cwd: |d| vec![joined("--working-dir=", d)],
        root: |_| vec![],
    },
    Terminal {
        prog: "xterm",
        cwd: |_| vec![],
        root: |_| {
            vec![
                OsString::from("-e"),
                OsString::from("sudo"),
                OsString::from("-s"),
            ]
        },
    },
];

/// `true` if `prog` is an executable in PATH.
fn which(prog: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    path.split(':').any(|dir| {
        let candidate = std::path::Path::new(dir).join(prog);
        use std::os::unix::fs::PermissionsExt as _;
        candidate
            .metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

/// Picks the first available terminal and builds `(program, args)`,
/// without spawning anything. Testable with a fake table.
fn select_command<'t>(
    dir: &Path,
    root: bool,
    terms: &'t [Terminal],
    mut available: impl FnMut(&str) -> bool,
) -> Result<(&'t str, Vec<OsString>), String> {
    for terminal in terms {
        if !available(terminal.prog) {
            continue;
        }
        let args = if root {
            (terminal.root)(dir)
        } else {
            (terminal.cwd)(dir)
        };
        if args.is_empty() {
            return Err(format!(
                "A root shell is not supported by {}",
                terminal.prog
            ));
        }
        return Ok((terminal.prog, args));
    }
    Err("No terminal emulator found".to_string())
}

/// Spawns `prog` with `args` in `dir`. Split out so argument building
/// is verifiable without spawning real terminals.
fn spawn_command(prog: &str, args: &[OsString], dir: &Path) -> Result<(), String> {
    Command::new(prog)
        .args(args)
        .current_dir(dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Opens the terminal in `dir`. With `root` the shell already starts
/// as root (`sudo -s` inside the terminal: `sudo -i` would `cd` into
/// root's home, `-s` stays in the current folder instead).
pub fn open(dir: &Path, root: bool) -> Result<(), String> {
    let (prog, args) = select_command(dir, root, &TERMINALS, which)?;
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
            root: |_| vec![],
        },
        Terminal {
            prog: "fake-term",
            cwd: |d| vec![joined("--dir=", d)],
            root: |d| {
                vec![
                    joined("--dir=", d),
                    OsString::from("-e"),
                    OsString::from("sudo -s"),
                ]
            },
        },
        Terminal {
            prog: "other-term",
            cwd: |d| vec![d.as_os_str().to_owned()],
            root: |d| vec![d.as_os_str().to_owned()],
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
    fn select_root_unsupported_errors() {
        let dir = Path::new("/tmp/x");
        let err = select_command(dir, true, &FAKE_TERMS, |p| p == "missing-term").unwrap_err();
        assert!(err.contains("missing-term"));
        assert!(select_command(dir, false, &FAKE_TERMS, |_| false).is_err());
    }

    #[test]
    fn select_preserves_non_utf8_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let raw = b"/tmp/bad \xff dir";
        let dir = Path::new(OsStr::from_bytes(raw));
        let (prog, args) = select_command(dir, false, &FAKE_TERMS, |p| p == "other-term").unwrap();
        assert_eq!(prog, "other-term");
        assert_eq!(args, vec![OsStr::from_bytes(raw).to_owned()]);
    }

    #[test]
    fn spawn_uses_workdir_without_real_terminal() {
        // `/bin/true` exits immediately: proves program + workdir wiring.
        let tmp = std::env::temp_dir();
        assert!(tmp.is_dir());
        assert!(spawn_command("/bin/true", &[], &tmp).is_ok());
        assert!(spawn_command("/bin/does-not-exist-kito", &[], &tmp).is_err());
    }
}
