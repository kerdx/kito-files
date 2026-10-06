//! Operazioni file pure su GIO, senza dipendenze GTK.
//! Testabile in isolamento: `cargo test -p kito-core`.

pub mod bookmarks;

use gio::prelude::*;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;

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
/// I collegamenti simbolici non vengono mai seguiti: eliminare un
/// symlink rimuove solo il collegamento, anche se punta a una
/// directory (esterna, interrotta o circolare).
pub fn delete_recursive(uri: &str) -> Result<(), glib::Error> {
    let file = gio::File::for_uri(uri);
    // NOFOLLOW_SYMLINKS: un symlink (a file o directory, esistente o
    // interrotto) ha tipo SymbolicLink e salta la ricorsione; la delete
    // finale rimuove il solo collegamento. Senza questo flag un symlink
    // a directory verrebbe attraversato, cancellando i file della
    // destinazione, e uno circolare ricorserebbe all'infinito.
    if file.query_file_type(
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        gio::Cancellable::NONE,
    ) == gio::FileType::Directory
    {
        // Enumerazione best-effort: su alcuni backend (cestino) l'elenco
        // dei figli può fallire; in quel caso si prova subito la delete.
        if let Ok(children) = file.enumerate_children(
            "standard::name",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            gio::Cancellable::NONE,
        ) {
            while let Some(info) = children.next_file(gio::Cancellable::NONE)? {
                delete_recursive(&children.child(&info).uri())?;
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

/// Riporta una voce del cestino nella sua posizione originale.
/// Il nome viene dal percorso originale (il basename nel cestino può
/// differire); se occupato, l'esistente è preservato con un suffisso.
/// Ritorna l'URI finale. In caso di errore l'elemento resta nel cestino.
pub fn restore(uri: &str) -> Result<String, glib::Error> {
    let file = gio::File::for_uri(uri);
    let info = file.query_info(
        gio::FILE_ATTRIBUTE_TRASH_ORIG_PATH.as_str(),
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    )?;
    let dest = restore_destination(&info)?;
    file.move_(
        &dest,
        gio::FileCopyFlags::NONE,
        gio::Cancellable::NONE,
        Some(&mut |_, _| {}),
    )
    .map(|_| dest.uri().into())
}

/// Legge un attributo byte-string GIO come byte grezzi, senza passare
/// dal getter gtk-rs (il cui debug assert pretende UTF-8 anche se il
/// contenuto è opaco: `trash::orig-path` è una byte string). Vale in
/// debug come in release.
fn file_info_byte_string(info: &gio::FileInfo, attr: &std::ffi::CStr) -> Option<Vec<u8>> {
    unsafe extern "C" {
        fn g_file_info_get_attribute_byte_string(
            info: *mut std::ffi::c_void,
            attribute: *const std::ffi::c_char,
        ) -> *const std::ffi::c_char;
    }
    let ptr =
        unsafe { g_file_info_get_attribute_byte_string(info.as_ptr() as *mut _, attr.as_ptr()) };
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { std::ffi::CStr::from_ptr(ptr) }.to_bytes().to_vec())
}

/// Destinazione di ripristino dai metadati del cestino: directory
/// originale + nome originale con suffisso se occupato. Il percorso
/// originale è una byte string (`trash::orig-path`, non
/// `standard::trash::orig-path`) e va letto come byte per preservare
/// spazi, caratteri speciali e nomi non UTF-8. Testabile senza backend.
fn restore_destination(info: &gio::FileInfo) -> Result<gio::File, glib::Error> {
    let bytes = file_info_byte_string(info, c"trash::orig-path")
        .filter(|b| !b.is_empty())
        .ok_or_else(|| io_error("Unknown original location"))?;
    let orig = std::path::PathBuf::from(OsStr::from_bytes(&bytes));
    let name = orig
        .file_name()
        .ok_or_else(|| io_error("Unknown original location"))?;
    let parent = orig
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| io_error("Unknown original location"))?;
    if !parent.is_dir() {
        return Err(io_error("Original location is no longer available"));
    }
    Ok(unique_child(&gio::File::for_path(parent), name))
}

/// Crea un file vuoto dentro `parent_dir_uri`. Se il nome è già occupato,
/// aggiunge un suffisso (`name (copy).ext`). Ritorna la nuova URI.
pub fn create_file(parent_dir_uri: &str, name: &str) -> Result<String, glib::Error> {
    let name = name.trim();
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(io_error("Invalid file name"));
    }
    let dir = gio::File::for_uri(parent_dir_uri);
    let candidate = unique_child(&dir, OsStr::new(name));
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

/// Divide `name` in (stelo, estensione) a livello di byte: l'ultimo
/// `.` non iniziale. Versione byte-safe di nomi non UTF-8.
fn stem_ext_os(name: &OsStr) -> (&OsStr, &OsStr) {
    let bytes = name.as_bytes();
    match bytes.iter().rposition(|&b| b == b'.') {
        Some(i) if i > 0 => {
            let (stem, ext) = bytes.split_at(i);
            (OsStr::from_bytes(stem), OsStr::from_bytes(ext))
        }
        _ => (name, OsStr::new("")),
    }
}

/// `dest_dir/name`, oppure `name (copy).ext`, `name (copy 2).ext`...
/// Opera su `OsStr` per preservare i nomi non UTF-8.
fn unique_child(dest_dir: &gio::File, name: &OsStr) -> gio::File {
    let mut candidate = dest_dir.child(name);
    if !candidate.query_exists(gio::Cancellable::NONE) {
        return candidate;
    }
    let (stem, ext) = stem_ext_os(name);
    let mut i = 1;
    loop {
        let mut numbered = OsString::from(stem);
        numbered.push(if i == 1 {
            " (copy)".to_string()
        } else {
            format!(" (copy {i})")
        });
        numbered.push(ext);
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

/// `true` se `dest_dir` è `src` o un suo discendente: copiarci dentro
/// una directory creerebbe la destinazione prima di enumerarla, e la
/// ricorsione la ricopierebbe all'infinito (copie annidate).
fn dest_inside_src(src: &gio::File, dest_dir: &gio::File) -> bool {
    // Percorsi locali: identità fisica via canonicalizzazione (risolve
    // `..`, i symlink nella destinazione stessa e nei componenti
    // intermedi). Niente prefissi testuali: `Path::starts_with` lavora
    // per componenti (`/tmp/A2` non è dentro `/tmp/A`).
    if let (Some(src_path), Some(dest_path)) = (src.path(), dest_dir.path()) {
        if let (Ok(src_canon), Ok(dest_canon)) = (
            std::fs::canonicalize(&src_path),
            std::fs::canonicalize(&dest_path),
        ) {
            return dest_canon.starts_with(&src_canon);
        }
        // Sorgente o destinazione non canonicalizzabile (mancante): si
        // lascia fallire la copia con il suo errore naturale.
        return false;
    }
    // Backend non locali (trash://, network://, ...): nessuna identità
    // fisica né symlink da risolvere; confronto sulle URI normalizzate
    // con guardia sul separatore. Limite: alias dello stesso oggetto con
    // URI diverse (o maiuscole diverse su backend case-insensitive) non
    // vengono rilevati.
    let norm = |uri: glib::GString| {
        let s = uri.to_string();
        s.trim_end_matches('/').to_string()
    };
    let s = norm(src.uri());
    let d = norm(dest_dir.uri());
    d == s || d.starts_with(&format!("{s}/"))
}

/// Copia file o cartella (ricorsiva) dentro `dest_dir_uri`.
/// Copiare una directory dentro sé stessa o in un suo discendente è
/// rifiutato prima di creare alcunché.
pub fn copy_to(src_uri: &str, dest_dir_uri: &str) -> Result<(), glib::Error> {
    let src = gio::File::for_uri(src_uri);
    let dest_dir = gio::File::for_uri(dest_dir_uri);
    if src.query_file_type(gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
        == gio::FileType::Directory
        && dest_inside_src(&src, &dest_dir)
    {
        return Err(io_error(
            "Cannot copy a folder into itself or one of its subfolders",
        ));
    }
    let name = src
        .basename()
        .and_then(|n| n.into_string().ok())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| io_error("Invalid file name"))?;
    let dest = unique_child(&gio::File::for_uri(dest_dir_uri), OsStr::new(&name));
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
    let dest = unique_child(&gio::File::for_uri(dest_dir_uri), OsStr::new(&name));
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

        // File normale: senza `trash::orig-path` non si ripristina.
        assert!(restore(&uri(&tmp.path().join("file.txt"))).is_err());
        assert!(tmp.path().join("file.txt").exists());
    }

    /// FileInfo con il solo attributo che conta, come lo dà il backend.
    fn trash_info(orig_path: &str) -> gio::FileInfo {
        let info = gio::FileInfo::new();
        info.set_attribute_byte_string(gio::FILE_ATTRIBUTE_TRASH_ORIG_PATH.as_str(), orig_path);
        info
    }

    #[test]
    fn restore_destination_reads_orig_path() {
        let tmp = tempfile::tempdir().unwrap();
        let orig = tmp.path().join("my doc (1) [x].txt");

        // Nome preso dal percorso originale, spazi e speciali intatti.
        let dest = restore_destination(&trash_info(orig.to_str().unwrap())).unwrap();
        assert_eq!(
            dest.uri().to_string(),
            gio::File::for_path(&orig).uri().to_string()
        );
    }

    #[test]
    fn restore_destination_collision_keeps_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let orig = tmp.path().join("report.txt");
        std::fs::write(&orig, b"existing").unwrap();

        let dest = restore_destination(&trash_info(orig.to_str().unwrap())).unwrap();
        assert!(
            dest.uri().to_string().ends_with("report%20(copy).txt")
                || dest.uri().to_string().ends_with("report (copy).txt")
        );
        // L'esistente non viene toccato: solo calcolato il nome libero.
        assert_eq!(std::fs::read(&orig).unwrap(), b"existing");
    }

    #[test]
    fn restore_destination_missing_metadata_errors() {
        // Nessun attributo.
        let err = restore_destination(&gio::FileInfo::new()).unwrap_err();
        assert!(err.to_string().contains("Unknown original location"));

        // Il vecchio nome attributo errato non viene più letto.
        let info = gio::FileInfo::new();
        info.set_attribute_byte_string("standard::trash::orig-path", "/tmp/x.txt");
        let err = restore_destination(&info).unwrap_err();
        assert!(err.to_string().contains("Unknown original location"));

        // Directory originale sparita.
        let missing = trash_info("/tmp/kito-definitely-gone-xyz/f.txt");
        let err = restore_destination(&missing).unwrap_err();
        assert!(err.to_string().contains("no longer available"));
    }

    #[test]
    fn stem_ext_os_handles_non_utf8() {
        use std::os::unix::ffi::OsStrExt;
        let name = OsStr::from_bytes(b"caf\xe9nota.txt");
        let (stem, ext) = stem_ext_os(name);
        assert_eq!(stem.as_bytes(), b"caf\xe9nota");
        assert_eq!(ext.as_bytes(), b".txt");
        let (stem, ext) = stem_ext_os(OsStr::new(".hidden"));
        assert_eq!(stem.as_bytes(), b".hidden");
        assert_eq!(ext.as_bytes(), b"");
        let (stem, ext) = stem_ext_os(OsStr::new("noext"));
        assert_eq!(stem.as_bytes(), b"noext");
        assert_eq!(ext.as_bytes(), b"");
    }

    /// Cestina `path` via GIO. Ritorna `None` (skip) se il backend non
    /// è disponibile in questo ambiente; non tocca altre voci.
    fn trash_tempfile(path: &std::path::Path) -> Option<()> {
        if let Err(e) = gio::File::for_path(path).trash(gio::Cancellable::NONE) {
            eprintln!("SKIP trash e2e: backend unavailable ({e})");
            return None;
        }
        assert!(!path.exists());
        Some(())
    }

    /// URI `trash:///` della voce la cui origine è `orig`, o panico.
    /// Confronta i byte grezzi (niente conversioni UTF-8: le altre voci
    /// del cestino possono avere nomi arbitrari). gvfsd nota le nuove
    /// voci con un monitor: breve retry prima di arrenderti.
    fn find_trash_entry(orig: &std::path::Path) -> String {
        use std::os::unix::ffi::OsStrExt;
        let want = orig.as_os_str().as_bytes();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let trash = gio::File::for_uri(TRASH_URI);
            let children = trash
                .enumerate_children(
                    "standard::name,trash::orig-path",
                    gio::FileQueryInfoFlags::NONE,
                    gio::Cancellable::NONE,
                )
                .unwrap();
            while let Some(info) = children.next_file(gio::Cancellable::NONE).unwrap() {
                if file_info_byte_string(&info, c"trash::orig-path").as_deref() == Some(want) {
                    return children.child(&info).uri().to_string();
                }
            }
            if std::time::Instant::now() >= deadline {
                panic!("trash entry not found for {}", orig.display());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    /// Temp dir sotto la home: /tmp sta su un mount dove il cestino non
    /// è supportato, la home sì. Solo elementi della prova, mai esistenti.
    fn home_tempdir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("kito-restore-test-")
            .tempdir_in(glib::home_dir().join(".cache"))
            .unwrap()
    }

    #[test]
    fn restore_roundtrip_through_trash() {
        // Nome voce unico per processo: riusare lo stesso nome in run
        // ravvicinati confonde il monitor di gvfsd-trash (eventi
        // coalescenti, la voce poi non appare più).
        let unique = format!(
            "round trip (1) {}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let tmp = home_tempdir();
        let path = tmp.path().join(&unique);
        std::fs::write(&path, b"payload").unwrap();
        let Some(()) = trash_tempfile(&path) else {
            return;
        };

        // Un solo ciclo trash->restore: cicli multipli ravvicinati sullo
        // stesso nome perdono eventi nel monitor di gvfsd-trash (la sua
        // cache diverge e la voce non appare più; problema del demone,
        // non del ripristino). La collisione è coperta a livello unit.
        let restored = restore(&find_trash_entry(&path)).unwrap();
        assert_eq!(restored, gio::File::for_path(&path).uri().to_string());
        assert_eq!(std::fs::read(&path).unwrap(), b"payload");
    }

    /// Imposta `trash::orig-path` a byte grezzi via C API diretta (il
    /// setter del binding accetta solo `&str`). Solo per i test.
    unsafe fn set_orig_path_raw(info: &gio::FileInfo, raw: &[u8]) {
        unsafe extern "C" {
            fn g_file_info_set_attribute_byte_string(
                info: *mut std::ffi::c_void,
                attribute: *const std::ffi::c_char,
                value: *const std::ffi::c_char,
            );
        }
        let path = std::ffi::CString::new(raw).unwrap();
        unsafe {
            g_file_info_set_attribute_byte_string(
                info.as_ptr() as *mut _,
                c"trash::orig-path".as_ptr(),
                path.as_ptr(),
            );
        }
    }

    #[test]
    fn restore_destination_non_utf8_orig_path() {
        use std::os::unix::ffi::OsStrExt;
        let tmp = tempfile::tempdir().unwrap();
        let mut raw = tmp.path().as_os_str().as_bytes().to_vec();
        raw.extend_from_slice(b"/weird \xff.txt");
        let info = gio::FileInfo::new();
        unsafe { set_orig_path_raw(&info, &raw) };

        let dest = restore_destination(&info).unwrap();
        assert_eq!(
            dest.uri().to_string(),
            gio::File::for_path(OsStr::from_bytes(&raw))
                .uri()
                .to_string()
        );
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
    fn delete_symlink_to_external_dir_keeps_target() {
        let external = tempfile::tempdir().unwrap();
        std::fs::write(external.path().join("keep.txt"), b"do not touch").unwrap();
        let home = tempfile::tempdir().unwrap();
        let link = home.path().join("link");
        std::os::unix::fs::symlink(external.path(), &link).unwrap();

        delete_recursive(&uri(&link)).unwrap();

        // Sparisce solo il collegamento: destinazione intatta.
        assert!(std::fs::symlink_metadata(&link).is_err());
        assert_eq!(
            std::fs::read(external.path().join("keep.txt")).unwrap(),
            b"do not touch"
        );
    }

    #[test]
    fn delete_dir_with_external_symlink_keeps_target() {
        let external = tempfile::tempdir().unwrap();
        std::fs::write(external.path().join("data.txt"), b"data").unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("inner.txt"), b"inner").unwrap();
        std::os::unix::fs::symlink(external.path(), dir.path().join("ext")).unwrap();

        delete_recursive(&uri(dir.path())).unwrap();

        assert!(!dir.path().exists());
        assert_eq!(
            std::fs::read(external.path().join("data.txt")).unwrap(),
            b"data"
        );
    }

    #[test]
    fn delete_broken_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("dangling");
        std::os::unix::fs::symlink(tmp.path().join("no-such-target"), &link).unwrap();
        assert!(std::fs::symlink_metadata(&link).is_ok());

        delete_recursive(&uri(&link)).unwrap();

        assert!(std::fs::symlink_metadata(&link).is_err());
    }

    #[test]
    fn delete_circular_symlink_does_not_recurse() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("file.txt"), b"x").unwrap();
        // Collegamento alla directory che lo contiene: seguirlo
        // ricorserebbe all'infinito.
        std::os::unix::fs::symlink(tmp.path(), tmp.path().join("loop")).unwrap();

        delete_recursive(&uri(tmp.path())).unwrap();

        assert!(!tmp.path().exists());
    }

    #[test]
    fn delete_plain_tree_is_recursive() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("deep.txt"), b"deep").unwrap();
        // Symlink a file dentro l'albero: si elimina col resto.
        std::os::unix::fs::symlink(
            nested.join("deep.txt"),
            tmp.path().join("a").join("file-link.txt"),
        )
        .unwrap();

        delete_recursive(&uri(&tmp.path().join("a"))).unwrap();

        assert!(!tmp.path().join("a").exists());
    }

    #[test]
    fn delete_missing_returns_error() {
        assert!(
            delete_recursive(&uri(&tempfile::tempdir().unwrap().path().join("ghost"))).is_err()
        );
    }

    /// Nomi ordinati delle voci di `dir`: istantanea per verificare che
    /// un'operazione rifiutata non crei nulla.
    fn names(dir: &std::path::Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    fn copy_fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("A");
        std::fs::create_dir_all(a.join("sub")).unwrap();
        std::fs::write(a.join("f.txt"), b"effe").unwrap();
        std::fs::write(a.join("sub").join("g.txt"), b"gi").unwrap();
        tmp
    }

    #[test]
    fn copy_dir_into_itself_is_rejected() {
        let tmp = copy_fixture();
        let a = tmp.path().join("A");
        let before = names(&a);

        assert!(copy_to(&uri(&a), &uri(&a)).is_err());

        // Nulla creato, sorgente intatta.
        assert_eq!(names(&a), before);
        assert_eq!(std::fs::read(a.join("f.txt")).unwrap(), b"effe");
        assert_eq!(std::fs::read(a.join("sub").join("g.txt")).unwrap(), b"gi");
    }

    #[test]
    fn copy_dir_into_subdir_is_rejected() {
        let tmp = copy_fixture();
        let a = tmp.path().join("A");
        let sub = a.join("sub");
        let before_a = names(&a);
        let before_sub = names(&sub);

        assert!(copy_to(&uri(&a), &uri(&sub)).is_err());

        assert_eq!(names(&a), before_a);
        assert_eq!(names(&sub), before_sub);
        assert_eq!(std::fs::read(a.join("f.txt")).unwrap(), b"effe");
    }

    #[test]
    fn copy_dir_through_symlink_dest_is_rejected() {
        let tmp = copy_fixture();
        let a = tmp.path().join("A");
        let before = names(&a);
        std::os::unix::fs::symlink(&a, tmp.path().join("link_a")).unwrap();
        std::os::unix::fs::symlink(a.join("sub"), tmp.path().join("link_sub")).unwrap();

        // Destinazione che raggiunge A o un suo discendente via symlink.
        assert!(copy_to(&uri(&a), &uri(&tmp.path().join("link_a"))).is_err());
        assert!(copy_to(&uri(&a), &uri(&tmp.path().join("link_sub"))).is_err());

        assert_eq!(names(&a), before);
        assert_eq!(std::fs::read(a.join("f.txt")).unwrap(), b"effe");
    }

    #[test]
    fn copy_between_siblings_with_prefix_names() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("A");
        let a2 = tmp.path().join("A2");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&a2).unwrap();
        std::fs::write(a.join("a.txt"), b"a").unwrap();
        std::fs::write(a2.join("b.txt"), b"b").unwrap();

        // `A2` non è dentro `A` nonostante il prefisso: copie valide.
        copy_to(&uri(&a2), &uri(&a)).unwrap();
        assert_eq!(std::fs::read(a.join("A2").join("b.txt")).unwrap(), b"b");
        copy_to(&uri(&a), &uri(&a2)).unwrap();
        assert_eq!(std::fs::read(a2.join("A").join("a.txt")).unwrap(), b"a");
        // Sorgenti intatte.
        assert_eq!(std::fs::read(a.join("a.txt")).unwrap(), b"a");
        assert_eq!(std::fs::read(a2.join("b.txt")).unwrap(), b"b");
    }

    #[test]
    fn copy_dir_collision_gets_copy_suffix() {
        let src = tempfile::tempdir().unwrap();
        std::fs::create_dir(src.path().join("docs")).unwrap();
        std::fs::write(src.path().join("docs").join("a.txt"), b"aaa").unwrap();
        let dest = tempfile::tempdir().unwrap();
        std::fs::create_dir(dest.path().join("docs")).unwrap();
        std::fs::write(dest.path().join("docs").join("other.txt"), b"o").unwrap();

        copy_to(&uri(&src.path().join("docs")), &uri(dest.path())).unwrap();

        assert_eq!(
            std::fs::read(dest.path().join("docs (copy)").join("a.txt")).unwrap(),
            b"aaa"
        );
        assert_eq!(
            std::fs::read(dest.path().join("docs").join("other.txt")).unwrap(),
            b"o"
        );
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
