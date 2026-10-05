//! Apertura del terminale di sistema nella cartella corrente.
//! GIO non espone i terminali: si cerca l'eseguibile in PATH e si usano
//! le opzioni `--working-directory` / `-e` note. La shell root gira
//! dentro il terminale (`sudo -i`): `pkexec` sanitizzerebbe l'ambiente
//! e il terminale non raggiungerebbe il display Wayland.

use std::process::Command;

/// Un emulatore di terminale noto: come aprirlo in una cartella e come
/// aprirvi una shell root.
struct Terminal {
    prog: &'static str,
    /// Argomenti per aprire il terminale in `dir`.
    cwd: fn(&str) -> Vec<String>,
    /// Argomenti per una shell root in `dir`; vuoto = non supportato.
    root: fn(&str) -> Vec<String>,
}

const TERMINALS: [Terminal; 10] = [
    Terminal {
        prog: "konsole",
        cwd: |d| vec!["--workdir".into(), d.into()],
        root: |d| {
            vec![
                "--workdir".into(),
                d.into(),
                "-e".into(),
                "sudo".into(),
                "-i".into(),
            ]
        },
    },
    Terminal {
        prog: "gnome-terminal",
        cwd: |d| vec![format!("--working-directory={d}")],
        root: |d| {
            vec![
                format!("--working-directory={d}"),
                "--".into(),
                "sudo".into(),
                "-i".into(),
            ]
        },
    },
    Terminal {
        prog: "kgx",
        cwd: |d| vec!["--working-directory".into(), d.into()],
        root: |d| {
            vec![
                "--working-directory".into(),
                d.into(),
                "--".into(),
                "sudo".into(),
                "-i".into(),
            ]
        },
    },
    Terminal {
        prog: "xfce4-terminal",
        cwd: |d| vec![format!("--working-directory={d}")],
        root: |d| {
            vec![
                format!("--working-directory={d}"),
                "-x".into(),
                "sudo".into(),
                "-i".into(),
            ]
        },
    },
    Terminal {
        prog: "tilix",
        cwd: |d| vec![format!("--working-directory={d}")],
        root: |d| {
            vec![
                format!("--working-directory={d}"),
                "-e".into(),
                "sudo -i".into(),
            ]
        },
    },
    Terminal {
        prog: "alacritty",
        cwd: |_| vec![],
        root: |_| vec!["-e".into(), "sudo".into(), "-i".into()],
    },
    Terminal {
        prog: "kitty",
        cwd: |_| vec![],
        root: |_| vec!["sudo".into(), "-i".into()],
    },
    Terminal {
        prog: "wezterm",
        cwd: |d| vec!["start".into(), "--cwd".into(), d.into()],
        root: |d| {
            vec![
                "start".into(),
                "--cwd".into(),
                d.into(),
                "--".into(),
                "sudo".into(),
                "-i".into(),
            ]
        },
    },
    Terminal {
        prog: "foot",
        cwd: |d| vec![format!("--working-dir={d}")],
        root: |_| vec![],
    },
    Terminal {
        prog: "xterm",
        cwd: |_| vec![],
        root: |_| vec!["-e".into(), "sudo".into(), "-i".into()],
    },
];

/// `true` se `prog` è un eseguibile in PATH.
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

/// Apre il terminale in `dir`. Con `root` la shell parte già root
/// (`sudo -i` dentro il terminale: nessuna sanitizzazione d'ambiente).
pub fn open(dir: &str, root: bool) -> Result<(), String> {
    for terminal in TERMINALS {
        if !which(terminal.prog) {
            continue;
        }
        let args = if root {
            (terminal.root)(dir)
        } else {
            (terminal.cwd)(dir)
        };
        if args.is_empty() {
            return Err(format!("A root shell is not supported by {}", terminal.prog));
        }
        return Command::new(terminal.prog)
            .args(&args)
            .current_dir(dir)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string());
    }
    Err("No terminal emulator found".to_string())
}
