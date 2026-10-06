//! Operazioni file pure su GIO, senza dipendenze GTK.
//! Testabile in isolamento: `cargo test -p kito-core`.

pub mod bookmarks;

use gio::prelude::*;

/// Una voce di directory: il minimo che serve alla UI per la vista classica.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Nome file (basename, non il path intero).
    pub name: String,
    /// URI `file://` completa, stabile anche con nomi strani.
    pub uri: String,
    /// `true` se directory (segue i symlink come fa GIO).
    pub is_dir: bool,
    /// Dimensione in byte, -1 se cartella o sconosciuta.
    pub size: i64,
    /// Tipo contenuto MIME (solo file), es. `text/plain`.
    pub content_type: Option<String>,
    /// Icona dal tema (tipo contenuto, cartelle speciali, symlink...).
    pub icon: Option<gio::Icon>,
}

/// Elenca il contenuto di `dir_uri` (es. `file:///home/utente`).
/// Restituisce le voci ordinate: prima le directory, poi i file, per nome.
/// Con `show_hidden = false` salta i file che iniziano per `.`.
pub fn list_dir(dir_uri: &str, show_hidden: bool) -> Result<Vec<Entry>, glib::Error> {
    let dir = gio::File::for_uri(dir_uri);
    let enumerator = dir.enumerate_children(
        "standard::name,standard::type,standard::size,standard::icon,standard::content-type",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    )?;

    let mut entries = Vec::new();
    while let Some(info) = enumerator.next_file(gio::Cancellable::NONE)? {
        let name = info.name();
        let name = name.to_string_lossy();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let file = enumerator.child(&info);
        let is_dir = info.file_type() == gio::FileType::Directory;
        entries.push(Entry {
            name: name.into_owned(),
            uri: file.uri().into(),
            is_dir,
            size: if is_dir { -1 } else { info.size() },
            content_type: (!is_dir)
                .then(|| info.content_type())
                .flatten()
                .map(|s| s.into()),
            icon: info.icon(),
        });
    }

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

fn io_error(msg: &str) -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::InvalidFilename, msg)
}

/// Sposta nel cestino (file o cartella). Niente conferma qui: la UI chiede.
pub fn trash(uri: &str) -> Result<(), glib::Error> {
    gio::File::for_uri(uri).trash(gio::Cancellable::NONE)
}

/// Cancellazione permanente ricorsiva. La UI chiede conferma prima.
pub fn delete_recursive(uri: &str) -> Result<(), glib::Error> {
    let file = gio::File::for_uri(uri);
    if file.query_file_type(gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
        == gio::FileType::Directory
    {
        // Enumerazione best-effort: su alcuni backend (cestino) l'elenco
        // dei figli può fallire; in quel caso si prova subito la delete.
        if let Ok(children) = file.enumerate_children(
            "standard::name",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        ) {
            while let Ok(Some(info)) = children.next_file(gio::Cancellable::NONE) {
                let _ = delete_recursive(&children.child(&info).uri());
            }
        }
    }
    file.delete(gio::Cancellable::NONE)
}

/// URI del cestino (backend GIO, richiede gvfs).
pub const TRASH_URI: &str = "trash:///";

/// Svuota `dir_uri`: elimina ogni voce contenuta. Ritorna
/// `(voci totali, voci non eliminate)`. Usato dal cestino (`empty_trash`).
pub fn empty_dir(dir_uri: &str) -> Result<(usize, usize), glib::Error> {
    let dir = gio::File::for_uri(dir_uri);
    let children = dir.enumerate_children(
        "standard::name",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    )?;
    // Prima l'elenco, poi le eliminazioni: cancellare mentre si itera
    // sui backend a lettura pigra nasconde le voci rimanenti.
    let mut uris = Vec::new();
    while let Some(info) = children.next_file(gio::Cancellable::NONE)? {
        uris.push(children.child(&info).uri().to_string());
    }
    drop(children);
    let mut failed = 0;
    for uri in &uris {
        if delete_recursive(uri).is_err() {
            failed += 1;
        }
    }
    Ok((uris.len(), failed))
}

/// Svuota il cestino: tutte le voci vengono eliminate definitivamente.
pub fn empty_trash() -> Result<(usize, usize), glib::Error> {
    empty_dir(TRASH_URI)
}

/// Riporta una voce del cestino nella sua posizione originale
/// (`standard::trash::orig-path`). Se il nome è già occupato, il file
/// viene comunque creato con un suffisso. Ritorna la URI finale.
pub fn restore(uri: &str) -> Result<String, glib::Error> {
    let file = gio::File::for_uri(uri);
    let info = file.query_info(
        "standard::trash::orig-path",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    )?;
    let orig = info
        .attribute_string("standard::trash::orig-path")
        .ok_or_else(|| io_error("Unknown original location"))?;
    let parent = gio::File::for_path(orig.as_str())
        .parent()
        .ok_or_else(|| io_error("Unknown original location"))?;
    let name = file
        .basename()
        .and_then(|n| n.into_string().ok())
        .unwrap_or_else(|| orig.to_string());
    let dest = unique_child(&parent, &name);
    file.move_(
        &dest,
        gio::FileCopyFlags::NONE,
        gio::Cancellable::NONE,
        Some(&mut |_, _| {}),
    )
    .map(|_| dest.uri().into())
}

/// Crea un file vuoto dentro `parent_dir_uri`. Se il nome è già occupato,
/// aggiunge un suffisso (`name (copy).ext`). Ritorna la nuova URI.
pub fn create_file(parent_dir_uri: &str, name: &str) -> Result<String, glib::Error> {
    let name = name.trim();
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(io_error("Invalid file name"));
    }
    let dir = gio::File::for_uri(parent_dir_uri);
    let mut candidate = dir.child(name);
    if candidate.query_exists(gio::Cancellable::NONE) {
        let (stem, ext) = stem_ext(name);
        let mut i = 1;
        loop {
            let n = if i == 1 {
                format!("{stem} (copy){ext}")
            } else {
                format!("{stem} (copy {i}){ext}")
            };
            candidate = dir.child(&n);
            if !candidate.query_exists(gio::Cancellable::NONE) {
                break;
            }
            i += 1;
        }
    }
    candidate.create(gio::FileCreateFlags::NONE, gio::Cancellable::NONE)?;
    Ok(candidate.uri().into())
}

/// Metadati per la finestra "Proprietà".
#[derive(Debug, Clone)]
pub struct Props {
    pub name: String,
    pub uri: String,
    pub is_dir: bool,
    pub size: i64,
    /// Secondi unix, `None` se sconosciuto.
    pub modified: Option<i64>,
    pub content_type: Option<String>,
}

/// Legge nome, tipo, dimensione, data e contenuto di `uri`.
pub fn props(uri: &str) -> Result<Props, glib::Error> {
    let file = gio::File::for_uri(uri);
    let info = file.query_info(
        "standard::name,standard::type,standard::size,time::modified,standard::content-type",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    )?;
    Ok(Props {
        name: info.name().to_string_lossy().into_owned(),
        uri: uri.to_string(),
        is_dir: info.file_type() == gio::FileType::Directory,
        size: info.size(),
        modified: info.attribute_uint64("time::modified").try_into().ok(),
        content_type: info.content_type().map(|s| s.to_string()),
    })
}

/// Rinomina. Ritorna la nuova URI.
pub fn rename(uri: &str, new_name: &str) -> Result<String, glib::Error> {
    let new_name = new_name.trim();
    if new_name.is_empty() || new_name.contains('/') {
        return Err(io_error("Invalid file name"));
    }
    let renamed = gio::File::for_uri(uri).set_display_name(new_name, gio::Cancellable::NONE)?;
    Ok(renamed.uri().into())
}

/// Crea la cartella `name` dentro `parent_dir_uri`. Ritorna la nuova URI.
/// Errore se il nome è vuoto, contiene `/`, è `.`/`..` o se esiste già.
pub fn mkdir(parent_dir_uri: &str, name: &str) -> Result<String, glib::Error> {
    let name = name.trim();
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(io_error("Invalid folder name"));
    }
    let child = gio::File::for_uri(parent_dir_uri).child(name);
    child.make_directory(gio::Cancellable::NONE)?;
    Ok(child.uri().into())
}

fn stem_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// `dest_dir/name`, oppure `name (copy).ext`, `name (copy 2).ext`...
fn unique_child(dest_dir: &gio::File, name: &str) -> gio::File {
    let mut candidate = dest_dir.child(name);
    if !candidate.query_exists(gio::Cancellable::NONE) {
        return candidate;
    }
    let (stem, ext) = stem_ext(name);
    let mut i = 1;
    loop {
        let numbered = if i == 1 {
            format!("{stem} (copy){ext}")
        } else {
            format!("{stem} (copy {i}){ext}")
        };
        candidate = dest_dir.child(&numbered);
        if !candidate.query_exists(gio::Cancellable::NONE) {
            return candidate;
        }
        i += 1;
    }
}

fn copy_recursive(src: &gio::File, dest: &gio::File) -> Result<(), glib::Error> {
    if src.query_file_type(gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
        == gio::FileType::Directory
    {
        dest.make_directory(gio::Cancellable::NONE)?;
        let children = src.enumerate_children(
            "standard::name",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )?;
        while let Some(info) = children.next_file(gio::Cancellable::NONE)? {
            copy_recursive(&children.child(&info), &dest.child(info.name()))?;
        }
        Ok(())
    } else {
        src.copy(
            dest,
            gio::FileCopyFlags::NONE,
            gio::Cancellable::NONE,
            Some(&mut |_, _| {}),
        )
        .map(|_| ())
    }
}

/// Copia file o cartella (ricorsiva) dentro `dest_dir_uri`.
pub fn copy_to(src_uri: &str, dest_dir_uri: &str) -> Result<(), glib::Error> {
    let src = gio::File::for_uri(src_uri);
    let name = src
        .basename()
        .and_then(|n| n.into_string().ok())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| io_error("Invalid file name"))?;
    let dest = unique_child(&gio::File::for_uri(dest_dir_uri), &name);
    copy_recursive(&src, &dest)
}

/// Sposta dentro `dest_dir_uri`. Su filesystem diversi può fallire:
/// la UI mostra l'errore (fallback copy+delete in arrivo).
pub fn move_to(src_uri: &str, dest_dir_uri: &str) -> Result<(), glib::Error> {
    let src = gio::File::for_uri(src_uri);
    let name = src
        .basename()
        .and_then(|n| n.into_string().ok())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| io_error("Invalid file name"))?;
    let dest = unique_child(&gio::File::for_uri(dest_dir_uri), &name);
    src.move_(
        &dest,
        gio::FileCopyFlags::NONE,
        gio::Cancellable::NONE,
        Some(&mut |_, _| {}),
    )
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_dir_orders_dirs_first() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("zebra-dir")).unwrap();
        std::fs::create_dir(tmp.path().join("alpha-dir")).unwrap();
        std::fs::write(tmp.path().join("file.txt"), b"ciao").unwrap();

        let uri = format!("file://{}", tmp.path().display());
        let entries = list_dir(&uri, true).unwrap();

        assert_eq!(entries.len(), 3);
        // Directory prima (ordinate), poi i file.
        assert!(entries[0].is_dir);
        assert!(entries[1].is_dir);
        assert!(!entries[2].is_dir);
        assert_eq!(entries[0].name, "alpha-dir");
        assert_eq!(entries[2].name, "file.txt");
    }

    #[test]
    fn list_dir_hides_dotfiles() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".hidden"), b"x").unwrap();
        std::fs::write(tmp.path().join("visible"), b"x").unwrap();
        let uri = format!("file://{}", tmp.path().display());
        assert_eq!(list_dir(&uri, false).unwrap().len(), 1);
        assert_eq!(list_dir(&uri, true).unwrap().len(), 2);
    }

    #[test]
    fn list_dir_missing_returns_error() {
        assert!(list_dir("file:///non/esiste/sicuramente", true).is_err());
    }

    fn tree_fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join("docs")).unwrap();
        std::fs::write(root.join("docs").join("a.txt"), b"aaa").unwrap();
        std::fs::write(root.join("top.txt"), b"top").unwrap();
        tmp
    }

    fn uri(path: &std::path::Path) -> String {
        format!("file://{}", path.display())
    }

    #[test]
    fn copy_dir_is_recursive() {
        let tmp = tree_fixture();
        let dest = tempfile::tempdir().unwrap();
        copy_to(&uri(&tmp.path().join("docs")), &uri(dest.path())).unwrap();
        assert_eq!(
            std::fs::read(dest.path().join("docs").join("a.txt")).unwrap(),
            b"aaa"
        );
    }

    #[test]
    fn copy_collision_gets_copy_suffix() {
        let tmp = tree_fixture();
        let dest = tempfile::tempdir().unwrap();
        let dest_uri = uri(dest.path());
        copy_to(&uri(&tmp.path().join("top.txt")), &dest_uri).unwrap();
        copy_to(&uri(&tmp.path().join("top.txt")), &dest_uri).unwrap();
        assert!(dest.path().join("top.txt").exists());
        assert!(dest.path().join("top (copy).txt").exists());
    }

    #[test]
    fn mkdir_creates_folder_and_validates_name() {
        let tmp = tempfile::tempdir().unwrap();
        let parent = uri(tmp.path());

        let made = mkdir(&parent, "Nuova cartella").unwrap();
        assert!(tmp.path().join("Nuova cartella").is_dir());
        assert!(made.ends_with("Nuova%20cartella") || made.ends_with("Nuova cartella"));

        // Esistente, vuoto, con slash, `.` e `..`: tutti errori.
        assert!(mkdir(&parent, "Nuova cartella").is_err());
        assert!(mkdir(&parent, "   ").is_err());
        assert!(mkdir(&parent, "a/b").is_err());
        assert!(mkdir(&parent, ".").is_err());
        assert!(mkdir(&parent, "..").is_err());
    }

    #[test]
    fn empty_dir_removes_all_entries() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("cartella")).unwrap();
        std::fs::write(tmp.path().join("cartella").join("f.txt"), b"x").unwrap();
        std::fs::write(tmp.path().join("file.txt"), b"x").unwrap();

        // Ricorsivo: la cartella piena e il file spariscono insieme.
        let (total, failed) = empty_dir(&uri(tmp.path())).unwrap();
        assert_eq!((total, failed), (2, 0));
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
        // Directory già vuota: zero voci, nessun errore.
        assert_eq!(empty_dir(&uri(tmp.path())).unwrap(), (0, 0));
    }

    #[test]
    fn restore_requires_a_trash_entry() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("file.txt"), b"x").unwrap();

        // File normale: senza `standard::trash::orig-path` non si ripristina.
        assert!(restore(&uri(&tmp.path().join("file.txt"))).is_err());
        assert!(tmp.path().join("file.txt").exists());
    }

    #[test]
    fn rename_and_delete_recursive() {
        let tmp = tree_fixture();
        let new_uri = rename(&uri(&tmp.path().join("top.txt")), "renamed.txt").unwrap();
        assert!(tmp.path().join("renamed.txt").exists());
        assert!(new_uri.ends_with("renamed.txt"));

        assert!(rename(&uri(&tmp.path().join("renamed.txt")), "  ").is_err());
        assert!(rename(&uri(&tmp.path().join("renamed.txt")), "a/b").is_err());

        delete_recursive(&uri(&tmp.path().join("docs"))).unwrap();
        assert!(!tmp.path().join("docs").exists());
    }

    #[test]
    fn create_file_makes_empty_file_and_uniquifies() {
        let tmp = tempfile::tempdir().unwrap();
        let parent = uri(tmp.path());

        let made = create_file(&parent, "note.txt").unwrap();
        assert!(made.ends_with("note.txt"));
        assert_eq!(std::fs::read(tmp.path().join("note.txt")).unwrap(), b"");

        // Stesso nome: suffisso, senza sovrascrivere.
        create_file(&parent, "note.txt").unwrap();
        assert!(tmp.path().join("note (copy).txt").exists());

        assert!(create_file(&parent, "  ").is_err());
        assert!(create_file(&parent, "a/b").is_err());
    }

    #[test]
    fn props_reports_name_size_and_type() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("f.txt"), b"12345").unwrap();
        std::fs::create_dir(tmp.path().join("d")).unwrap();

        let p = props(&uri(&tmp.path().join("f.txt"))).unwrap();
        assert_eq!(p.name, "f.txt");
        assert!(!p.is_dir);
        assert_eq!(p.size, 5);
        assert!(p.modified.is_some());

        let d = props(&uri(&tmp.path().join("d"))).unwrap();
        assert!(d.is_dir);
    }
}
