//! Pure GIO file operations, no GTK dependencies.
//! Testable in isolation: `cargo test -p kito-core`.

pub mod bookmarks;

use gio::prelude::*;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;

/// A directory entry: the minimum the UI needs for the classic view.
#[derive(Debug, Clone)]
pub struct Entry {
    /// File name (basename, not the full path).
    pub name: String,
    /// Full `file://` URI, stable even with weird names.
    pub uri: String,
    /// `true` if directory (follows symlinks like GIO does).
    pub is_dir: bool,
    /// Size in bytes, -1 if folder or unknown.
    pub size: i64,
    /// Unix seconds from `time::modified`, if the backend provides it.
    pub modified: Option<i64>,
    /// MIME content type (files only), e.g. `text/plain`.
    pub content_type: Option<String>,
    /// Theme icon in `g_icon_to_string` form (parse back with
    /// `gio::Icon::for_string`). Plain strings keep `Entry` thread-safe
    /// so enumeration can run in a worker; parsing is lossless for the
    /// icons enumerators return, and unparsable values fall back to the
    /// generic icon in the view.
    pub icon: Option<String>,
}

#[cfg(test)]
mod sorting_tests {
    use super::*;

    fn entry(
        name: &str,
        uri: &str,
        is_dir: bool,
        size: i64,
        modified: Option<i64>,
        content_type: Option<&str>,
    ) -> Entry {
        Entry {
            name: name.to_string(),
            uri: uri.to_string(),
            is_dir,
            size,
            modified,
            content_type: content_type.map(str::to_string),
            icon: None,
        }
    }

    #[test]
    fn name_order_is_natural_and_handles_unbounded_numeric_runs() {
        let run_300 = format!("file{}", "9".repeat(300));
        let run_255 = format!("file{}", "9".repeat(255));
        let mut entries = vec![
            entry(&run_300, "u4", false, 0, None, None),
            entry(&run_255, "u3", false, 0, None, None),
            entry("file10", "u2", false, 0, None, None),
            entry("file2", "u1", false, 0, None, None),
        ];
        sort_entries(&mut entries, SortOrder::default());
        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["file2", "file10", run_255.as_str(), run_300.as_str()]
        );
    }

    #[test]
    fn directories_stay_first_and_missing_metadata_stays_last_in_both_directions() {
        for direction in [SortDirection::Ascending, SortDirection::Descending] {
            let mut entries = vec![
                entry("unknown", "u4", false, -1, None, None),
                entry("file2", "u3", false, 2, Some(2), Some("text/plain")),
                entry("folder", "u1", true, -1, None, None),
                entry("file1", "u2", false, 1, Some(1), Some("text/plain")),
            ];
            sort_entries(
                &mut entries,
                SortOrder {
                    field: SortField::Modified,
                    direction,
                },
            );
            assert!(entries[0].is_dir);
            assert_eq!(entries[3].name, "unknown");
        }
    }

    #[test]
    fn equal_keys_use_name_then_uri_for_deterministic_order() {
        let mut entries = vec![
            entry("same", "uri-b", false, 4, Some(1), None),
            entry("same", "uri-a", false, 4, Some(1), None),
        ];
        sort_entries(
            &mut entries,
            SortOrder {
                field: SortField::Size,
                direction: SortDirection::Descending,
            },
        );
        assert_eq!(entries[0].uri, "uri-a");
        assert_eq!(entries[1].uri, "uri-b");
    }

    #[test]
    fn cached_sort_keys_match_the_shared_comparator_for_all_orders() {
        let original = vec![
            entry("file10", "uri-z", false, 10, Some(3), Some("text/plain")),
            entry("file2", "uri-c", false, 2, Some(1), Some("text/plain")),
            entry("file02", "uri-b", false, 2, Some(1), Some("text/plain")),
            entry("folder2", "uri-a", true, -1, None, None),
            entry("unknown", "uri-d", false, -1, None, None),
            entry("image", "uri-e", false, 8, Some(2), Some("image/png")),
        ];

        for field in [
            SortField::Name,
            SortField::Size,
            SortField::Type,
            SortField::Modified,
        ] {
            for direction in [SortDirection::Ascending, SortDirection::Descending] {
                let order = SortOrder { field, direction };
                let mut expected = original.clone();
                expected.sort_by(|left, right| compare_entries(left, right, order));
                let mut actual = original.clone();
                sort_entries(&mut actual, order);
                assert_eq!(
                    actual.iter().map(|entry| &entry.uri).collect::<Vec<_>>(),
                    expected.iter().map(|entry| &entry.uri).collect::<Vec<_>>(),
                    "cached key mismatch for {field:?} {direction:?}"
                );
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortField {
    #[default]
    Name,
    Size,
    Type,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortDirection {
    #[default]
    Ascending,
    Descending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SortOrder {
    pub field: SortField,
    pub direction: SortDirection,
}

/// Timings for the two independent stages of directory listing. These are
/// useful for separating backend enumeration from in-memory ordering; they
/// say nothing about how quickly a particular display paints the result.
#[derive(Debug, Clone, Copy, Default)]
pub struct ListTimings {
    pub enumeration: std::time::Duration,
    pub sorting: std::time::Duration,
}

/// One entry considered while emptying a directory. A missing error means
/// deletion succeeded; failed URIs and backend causes remain paired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteResult {
    pub uri: String,
    pub error: Option<glib::Error>,
}

/// Lists the contents of `dir_uri` (e.g. `file:///home/user`).
/// Returns sorted entries: directories first, then files, by name.
/// With `show_hidden = false` skips files starting with `.`.
pub fn list_dir(dir_uri: &str, show_hidden: bool) -> Result<Vec<Entry>, glib::Error> {
    let cancellable = gio::Cancellable::new();
    list_dir_with_cancellable(dir_uri, show_hidden, &cancellable).map(|(entries, _)| entries)
}

/// Cancellable directory listing for worker threads. GIO enumeration checks
/// the cancellable between backend reads; sorting itself is a single stable
/// sort and can only be cancelled immediately after it finishes.
pub fn list_dir_with_cancellable(
    dir_uri: &str,
    show_hidden: bool,
    cancellable: &gio::Cancellable,
) -> Result<(Vec<Entry>, ListTimings), glib::Error> {
    list_dir_with_sort(dir_uri, show_hidden, cancellable, SortOrder::default())
}

/// Lists a directory with a selected sort order. GIO metadata and sorting
/// both run on the caller's worker thread.
pub fn list_dir_with_sort(
    dir_uri: &str,
    show_hidden: bool,
    cancellable: &gio::Cancellable,
    order: SortOrder,
) -> Result<(Vec<Entry>, ListTimings), glib::Error> {
    let dir = gio::File::for_uri(dir_uri);
    let enumeration_started = std::time::Instant::now();
    let enumerator = dir.enumerate_children(
        "standard::name,standard::type,standard::size,time::modified,standard::icon,standard::content-type",
        gio::FileQueryInfoFlags::NONE,
        Some(cancellable),
    )?;

    let mut entries = Vec::new();
    while let Some(info) = enumerator.next_file(Some(cancellable))? {
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
            modified: info.attribute_uint64("time::modified").try_into().ok(),
            content_type: (!is_dir)
                .then(|| info.content_type())
                .flatten()
                .map(|s| s.into()),
            icon: info
                .icon()
                .and_then(|icon| icon.to_string())
                .map(|s| s.to_string()),
        });
    }

    let enumeration = enumeration_started.elapsed();
    if cancellable.is_cancelled() {
        return Err(glib::Error::new(
            gio::IOErrorEnum::Cancelled,
            "Operation cancelled",
        ));
    }
    let sorting_started = std::time::Instant::now();
    sort_entries(&mut entries, order);
    let sorting = sorting_started.elapsed();
    if cancellable.is_cancelled() {
        return Err(glib::Error::new(
            gio::IOErrorEnum::Cancelled,
            "Operation cancelled",
        ));
    }
    Ok((
        entries,
        ListTimings {
            enumeration,
            sorting,
        },
    ))
}

/// Sorts entries deterministically. Directories remain first in both
/// directions; unknown metadata remains last within each group.
pub fn sort_entries(entries: &mut [Entry], order: SortOrder) {
    let name_keys: Vec<Vec<u8>> = entries
        .iter()
        .map(|entry| natural_name_sort_key(&entry.name))
        .collect();
    let mut indices: Vec<usize> = (0..entries.len()).collect();
    indices.sort_by(|left, right| {
        compare_entries_with_name_keys(
            &entries[*left],
            &entries[*right],
            &name_keys[*left],
            &name_keys[*right],
            order,
        )
    });

    // Apply the computed order in place; the index map keeps URI and icon
    // strings from being cloned into a second listing.
    let mut positions = vec![0; indices.len()];
    for (new_index, old_index) in indices.into_iter().enumerate() {
        positions[old_index] = new_index;
    }
    for index in 0..positions.len() {
        while positions[index] != index {
            let target = positions[index];
            entries.swap(index, target);
            positions.swap(index, target);
        }
    }
}

/// Queries one directory entry's display and sort metadata. Intended for
/// worker threads, including targeted updates from GIO monitors.
pub fn query_entry(uri: &str) -> Result<Entry, glib::Error> {
    query_entry_with_cancellable(uri, &gio::Cancellable::new())
}

pub fn query_entry_with_cancellable(
    uri: &str,
    cancellable: &gio::Cancellable,
) -> Result<Entry, glib::Error> {
    let file = gio::File::for_uri(uri);
    let info = file.query_info(
        "standard::name,standard::type,standard::size,time::modified,standard::icon,standard::content-type",
        gio::FileQueryInfoFlags::NONE,
        Some(cancellable),
    )?;
    let is_dir = info.file_type() == gio::FileType::Directory;
    Ok(Entry {
        name: info.name().to_string_lossy().into_owned(),
        uri: file.uri().to_string(),
        is_dir,
        size: if is_dir { -1 } else { info.size() },
        modified: info.attribute_uint64("time::modified").try_into().ok(),
        content_type: (!is_dir)
            .then(|| info.content_type())
            .flatten()
            .map(|content_type| content_type.to_string()),
        icon: info
            .icon()
            .and_then(|icon| icon.to_string())
            .map(|icon| icon.to_string()),
    })
}

/// Builds a virtual listing from explicit `(label, real URI)` items. Missing
/// targets remain visible and removable; no virtual URI reaches GIO.
pub fn list_uri_entries(
    items: &[(String, String)],
    order: SortOrder,
    cancellable: &gio::Cancellable,
) -> Result<(Vec<Entry>, ListTimings), glib::Error> {
    let started = std::time::Instant::now();
    let mut entries = Vec::with_capacity(items.len());
    for (label, uri) in items {
        if cancellable.is_cancelled() {
            return Err(glib::Error::new(
                gio::IOErrorEnum::Cancelled,
                "Operation cancelled",
            ));
        }
        let entry = match query_entry_with_cancellable(uri, cancellable) {
            Ok(entry) => entry,
            Err(error)
                if error.matches(gio::IOErrorEnum::Cancelled) || cancellable.is_cancelled() =>
            {
                return Err(error);
            }
            Err(_) => Entry {
                name: label.clone(),
                uri: uri.clone(),
                is_dir: false,
                size: -1,
                modified: None,
                content_type: None,
                icon: Some("text-x-generic".to_string()),
            },
        };
        entries.push(entry);
    }
    let enumeration = started.elapsed();
    let sort_started = std::time::Instant::now();
    sort_entries(&mut entries, order);
    let sorting = sort_started.elapsed();
    if cancellable.is_cancelled() {
        return Err(glib::Error::new(
            gio::IOErrorEnum::Cancelled,
            "Operation cancelled",
        ));
    }
    Ok((
        entries,
        ListTimings {
            enumeration,
            sorting,
        },
    ))
}

pub fn compare_entries(left: &Entry, right: &Entry, order: SortOrder) -> std::cmp::Ordering {
    let left_name = natural_name_sort_key(&left.name);
    let right_name = natural_name_sort_key(&right.name);
    compare_entries_with_name_keys(left, right, &left_name, &right_name, order)
}

fn compare_entries_with_name_keys(
    left: &Entry,
    right: &Entry,
    left_name: &[u8],
    right_name: &[u8],
    order: SortOrder,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match right.is_dir.cmp(&left.is_dir) {
        Ordering::Equal => {}
        other => return other,
    }

    let key = match order.field {
        SortField::Name => match order.direction {
            SortDirection::Ascending => left_name.cmp(right_name),
            SortDirection::Descending => right_name.cmp(left_name),
        },
        SortField::Size => compare_optional(
            (left.size >= 0).then_some(left.size),
            (right.size >= 0).then_some(right.size),
            order.direction,
        ),
        SortField::Type => {
            let left_type = if left.is_dir {
                Some("inode/directory")
            } else {
                left.content_type.as_deref()
            };
            let right_type = if right.is_dir {
                Some("inode/directory")
            } else {
                right.content_type.as_deref()
            };
            compare_optional(left_type, right_type, order.direction)
        }
        SortField::Modified => compare_optional(left.modified, right.modified, order.direction),
    };
    key.then_with(|| left_name.cmp(right_name))
        .then_with(|| left.uri.cmp(&right.uri))
}

fn compare_optional<T: Ord>(
    left: Option<T>,
    right: Option<T>,
    direction: SortDirection,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (left, right) {
        (Some(left), Some(right)) => match direction {
            SortDirection::Ascending => left.cmp(&right),
            SortDirection::Descending => right.cmp(&left),
        },
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Encodes a natural, case-insensitive name key for bytewise comparison.
/// Numeric runs use fixed-width length prefixes, so arbitrarily long digit
/// sequences sort naturally without parsing into a machine integer.
fn natural_name_sort_key(name: &str) -> Vec<u8> {
    let folded = name.to_lowercase();
    let bytes = folded.as_bytes();
    let mut key = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            key.push(bytes[index]);
            index += 1;
            continue;
        }

        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        let mut significant = start;
        while significant + 1 < index && bytes[significant] == b'0' {
            significant += 1;
        }

        // This marker sorts between non-digit bytes and ASCII digit bytes.
        key.push(b'0');
        append_ordered_len(&mut key, index - significant);
        key.extend_from_slice(&bytes[significant..index]);
        append_ordered_len(&mut key, index - start);
    }
    key
}

fn append_ordered_len(key: &mut Vec<u8>, length: usize) {
    if length < u8::MAX as usize {
        key.push(length as u8);
        return;
    }

    let bytes = length.to_be_bytes();
    let first_significant = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len() - 1);
    let significant = &bytes[first_significant..];
    key.push(u8::MAX);
    key.push(significant.len() as u8);
    key.extend_from_slice(significant);
}

fn io_error(msg: &str) -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::InvalidFilename, msg)
}

/// Moves to trash (file or folder). No confirmation here: the UI asks.
pub fn trash(uri: &str) -> Result<(), glib::Error> {
    gio::File::for_uri(uri).trash(gio::Cancellable::NONE)
}

/// Backend identity required before an operation can safely be undone. Some
/// virtual/remote filesystems do not expose both fields; those objects fail
/// closed rather than receiving a weaker path-only identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileIdentity {
    pub filesystem: String,
    pub file: String,
}

pub fn file_identity(uri: &str) -> Result<FileIdentity, glib::Error> {
    file_identity_with_cancellable(uri, gio::Cancellable::NONE)
}

/// Backend identity query with caller-controlled cancellation.
pub fn file_identity_cancellable(
    uri: &str,
    cancellable: &gio::Cancellable,
) -> Result<FileIdentity, glib::Error> {
    file_identity_with_cancellable(uri, Some(cancellable))
}

fn file_identity_with_cancellable(
    uri: &str,
    cancellable: Option<&gio::Cancellable>,
) -> Result<FileIdentity, glib::Error> {
    let info = gio::File::for_uri(uri).query_info(
        "id::filesystem,id::file",
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        cancellable,
    )?;
    let filesystem = info.attribute_string("id::filesystem");
    let file = info.attribute_string("id::file");
    match (filesystem, file) {
        (Some(filesystem), Some(file)) if !filesystem.is_empty() && !file.is_empty() => {
            Ok(FileIdentity {
                filesystem: filesystem.to_string(),
                file: file.to_string(),
            })
        }
        _ => Err(io_error(
            "Filesystem does not provide a stable file identity",
        )),
    }
}

/// Cancellable move into `dest_dir_uri` with the exact effective target
/// (see [`move_to_exact_result`]). Used by the F08 job center only.
pub fn move_to_exact_result_cancellable(
    src_uri: &str,
    dest_dir_uri: &str,
    cancellable: &gio::Cancellable,
) -> Result<String, glib::Error> {
    let src = gio::File::for_uri(src_uri);
    let dest_dir = gio::File::for_uri(dest_dir_uri);
    if src.query_file_type(gio::FileQueryInfoFlags::NONE, Some(cancellable))
        == gio::FileType::Directory
        && dest_inside_src(&src, &dest_dir)
    {
        return Err(io_error(
            "Cannot move a folder into itself or one of its subfolders",
        ));
    }
    if src.parent().is_some_and(|parent| parent.equal(&dest_dir)) {
        return Err(io_error("Source is already in the destination folder"));
    }
    let name = src
        .basename()
        .and_then(|n| n.into_string().ok())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| io_error("Invalid file name"))?;
    let dest = unique_child_cancellable(&dest_dir, OsStr::new(&name), cancellable)?;
    src.move_(
        &dest,
        gio::FileCopyFlags::NONE,
        Some(cancellable),
        Some(&mut |_, _| {}),
    )
    .map_err(|error| move_error_with_partial_target(error, &dest))?;
    Ok(dest.uri().to_string())
}

fn move_error_with_partial_target(error: glib::Error, dest: &gio::File) -> glib::Error {
    // GIO may implement a cross-filesystem move as copy+delete. A target
    // already present due to a collision is not attributable to this move.
    if error.matches(gio::IOErrorEnum::Exists) {
        return error;
    }
    let target_exists = dest
        .query_info(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            gio::Cancellable::NONE,
        )
        .is_ok();
    if !target_exists {
        return error;
    }

    let message = format!(
        "{error}; partial move output may remain at {}",
        dest.uri()
    );
    if error.matches(gio::IOErrorEnum::Cancelled) {
        glib::Error::new(gio::IOErrorEnum::Cancelled, &message)
    } else {
        glib::Error::new(gio::IOErrorEnum::Failed, &message)
    }
}

/// Moves to a directory and returns the exact effective URI (including any
/// collision suffix). Kept separate from `move_to` to preserve its API.
pub fn move_to_exact_result(src_uri: &str, dest_dir_uri: &str) -> Result<String, glib::Error> {
    let src = gio::File::for_uri(src_uri);
    let dest_dir = gio::File::for_uri(dest_dir_uri);
    if src.query_file_type(gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
        == gio::FileType::Directory
        && dest_inside_src(&src, &dest_dir)
    {
        return Err(io_error(
            "Cannot move a folder into itself or one of its subfolders",
        ));
    }
    if src.parent().is_some_and(|parent| parent.equal(&dest_dir)) {
        return Err(io_error("Source is already in the destination folder"));
    }
    let name = src
        .basename()
        .and_then(|n| n.into_string().ok())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| io_error("Invalid file name"))?;
    let dest = unique_child(&dest_dir, OsStr::new(&name));
    src.move_(
        &dest,
        gio::FileCopyFlags::NONE,
        gio::Cancellable::NONE,
        Some(&mut |_, _| {}),
    )?;
    Ok(dest.uri().to_string())
}

/// Exact inverse used by Undo: checks object identity and destination vacancy,
/// then uses GIO move semantics without selecting a collision suffix.
pub fn move_exact_checked(
    src_uri: &str,
    dest_uri: &str,
    expected: &FileIdentity,
) -> Result<(), glib::Error> {
    if file_identity(src_uri)? != *expected {
        return Err(io_error("File identity changed"));
    }
    let dest = gio::File::for_uri(dest_uri);
    if dest.query_exists(gio::Cancellable::NONE) {
        return Err(io_error("Undo destination is already occupied"));
    }
    gio::File::for_uri(src_uri).move_(
        &dest,
        gio::FileCopyFlags::NONE,
        gio::Cancellable::NONE,
        Some(&mut |_, _| {}),
    )
}

/// Cancellable trash with matching identity capture (see
/// [`trash_with_identity`]). Used by the F08 job center only.
pub fn trash_with_identity_cancellable(
    uri: &str,
    cancellable: &gio::Cancellable,
) -> Result<Option<(String, FileIdentity)>, glib::Error> {
    let identity = match file_identity_cancellable(uri, cancellable) {
        Ok(identity) => Some(identity),
        Err(error) if error.matches(gio::IOErrorEnum::Cancelled) => return Err(error),
        Err(_) => None,
    };
    let original = gio::File::for_uri(uri).path();
    gio::File::for_uri(uri).trash(Some(cancellable))?;
    let metadata = (|| {
        let (Some(identity), Some(original)) = (identity, original) else {
            return Ok(None);
        };
        let wanted = original.as_os_str().as_bytes();
        let children = gio::File::for_uri(TRASH_URI).enumerate_children(
            "standard::name,trash::orig-path,id::filesystem,id::file",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )?;
        let mut matches = Vec::new();
        loop {
            let info = match children.next_file(gio::Cancellable::NONE) {
                Ok(Some(info)) => info,
                Ok(None) => break,
                Err(error) => return Err(error),
            };
            let candidate = children.child(&info);
            if file_info_byte_string(&info, c"trash::orig-path").as_deref() == Some(wanted)
                && file_identity(candidate.uri().as_ref()).as_ref() == Ok(&identity)
            {
                matches.push(candidate.uri().to_string());
            }
        }
        Ok((matches.len() == 1).then(|| (matches.remove(0), identity)))
    })();
    // Trash already succeeded. Metadata is only needed for Undo and must
    // never turn a completed side effect into a reported operation failure.
    Ok(trash_metadata_after_success(metadata))
}

fn trash_metadata_after_success(
    metadata: Result<Option<(String, FileIdentity)>, glib::Error>,
) -> Option<(String, FileIdentity)> {
    metadata.ok().flatten()
}

/// Trash an item only retaining an undo target when a unique Trash entry can
/// be matched by original-path metadata and stable backend identity.
pub fn trash_with_identity(uri: &str) -> Result<Option<(String, FileIdentity)>, glib::Error> {
    let identity = file_identity(uri).ok();
    let original = gio::File::for_uri(uri).path();
    gio::File::for_uri(uri).trash(gio::Cancellable::NONE)?;
    let (Some(identity), Some(original)) = (identity, original) else {
        return Ok(None);
    };
    let wanted = original.as_os_str().as_bytes();
    let Ok(children) = gio::File::for_uri(TRASH_URI).enumerate_children(
        "standard::name,trash::orig-path,id::filesystem,id::file",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    ) else {
        return Ok(None);
    };
    let mut matches = Vec::new();
    loop {
        let info = match children.next_file(gio::Cancellable::NONE) {
            Ok(Some(info)) => info,
            Ok(None) => break,
            Err(_) => return Ok(None),
        };
        let candidate = children.child(&info);
        if file_info_byte_string(&info, c"trash::orig-path").as_deref() == Some(wanted)
            && file_identity(candidate.uri().as_ref()).as_ref() == Ok(&identity)
        {
            matches.push(candidate.uri().to_string());
        }
    }
    Ok((matches.len() == 1).then(|| (matches.remove(0), identity)))
}

/// Strict Trash restore for Undo: original location must be free, and the
/// candidate must retain the captured identity and exact original-path.
pub fn restore_exact_checked(
    uri: &str,
    original_uri: &str,
    expected: &FileIdentity,
) -> Result<(), glib::Error> {
    if file_identity(uri)? != *expected {
        return Err(io_error("File identity changed"));
    }
    let original = gio::File::for_uri(original_uri);
    if original.query_exists(gio::Cancellable::NONE) {
        return Err(io_error("Undo destination is already occupied"));
    }
    let info = gio::File::for_uri(uri).query_info(
        gio::FILE_ATTRIBUTE_TRASH_ORIG_PATH.as_str(),
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    )?;
    let path = original
        .path()
        .ok_or_else(|| io_error("Non-local Trash origin cannot be verified"))?;
    if file_info_byte_string(&info, c"trash::orig-path").as_deref()
        != Some(path.as_os_str().as_bytes())
    {
        return Err(io_error("Trash origin changed"));
    }
    gio::File::for_uri(uri).move_(
        &original,
        gio::FileCopyFlags::NONE,
        gio::Cancellable::NONE,
        Some(&mut |_, _| {}),
    )
}

/// Cancellable permanent recursive deletion (F08). Same NOFOLLOW_SYMLINKS
/// semantics as [`delete_recursive`].
pub fn delete_recursive_cancellable(
    uri: &str,
    cancellable: &gio::Cancellable,
) -> Result<(), glib::Error> {
    let file = gio::File::for_uri(uri);
    if file.query_file_type(gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, Some(cancellable))
        == gio::FileType::Directory
    {
        let children = file.enumerate_children(
            "standard::name",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            Some(cancellable),
        )?;
        while let Some(info) = children.next_file(Some(cancellable))? {
            delete_recursive_cancellable(&children.child(&info).uri(), cancellable)?;
        }
    }
    file.delete(Some(cancellable))
}

/// Permanent recursive deletion. The UI asks for confirmation first.
/// Symbolic links are never followed: deleting a
/// symlink removes only the link, even if it points to a
/// directory (external, broken or circular).
pub fn delete_recursive(uri: &str) -> Result<(), glib::Error> {
    let file = gio::File::for_uri(uri);
    // NOFOLLOW_SYMLINKS: a symlink (to file or directory, existing or
    // broken) has type SymbolicLink and skips recursion; the final
    // delete removes only the link. Without this flag a symlink
    // to a directory would be traversed, deleting the target's
    // files, and a circular one would recurse forever.
    if file.query_file_type(
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        gio::Cancellable::NONE,
    ) == gio::FileType::Directory
    {
        // Best-effort enumeration: on some backends (trash) listing
        // children may fail; then straight to delete.
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

/// Trash URI (GIO backend, requires gvfs).
pub const TRASH_URI: &str = "trash:///";

/// Stable virtual root for Freedesktop bookmarks; never pass to file operations.
pub const FAVORITES_URI: &str = "favorites:///";
/// Internal DND destination marker; never a GIO folder.
pub const ADD_BOOKMARK_URI: &str = "bookmark-add:///";

/// Local path of a `file://` URI, decoded via GIO (`%20`, Unicode,
/// `#`, ...). `None` for non-local URIs (`trash:///`, `network:///`,
/// ...). Never decodes manually: no double decoding, no mangled
/// literal `%` sequences.
pub fn uri_to_path(uri: &str) -> Option<std::path::PathBuf> {
    gio::File::for_uri(uri).path()
}

/// Readable text for a URI in the path bar and breadcrumbs: the decoded
/// local path, or the URI as-is when non-local. Non-UTF-8 names are
/// shown lossy: the exact round-trip is guaranteed by
/// [`resolve_path_text`], not by the text.
pub fn uri_to_display(uri: &str) -> String {
    match uri_to_path(uri) {
        Some(path) => path.to_string_lossy().into_owned(),
        None => uri.to_string(),
    }
}

/// Decoded base name of a URI (breadcrumb labels).
/// `None` when missing (e.g. the root, which has its own label).
pub fn uri_file_name(uri: &str) -> Option<String> {
    gio::File::for_uri(uri)
        .basename()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Resolves path-bar text into the URI to load:
/// - text unchanged from what was shown -> the stored URI (exact
///   round-trip even for lossy-shown non-UTF-8 paths);
/// - text with `://` -> treated as URI (including non-local ones);
/// - otherwise -> local path. Empty text -> `None`.
pub fn resolve_path_text(text: &str, shown_text: &str, shown_uri: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text == shown_text {
        return Some(shown_uri.to_string());
    }
    if text.contains("://") {
        return Some(text.to_string());
    }
    Some(gio::File::for_path(text).uri().to_string())
}

/// Empties `dir_uri`: deletes every contained entry. Returns
/// `(total entries, entries not deleted)`. Used by trash (`empty_trash`).
pub fn empty_dir(dir_uri: &str) -> Result<(usize, usize), glib::Error> {
    let results = empty_dir_detailed(dir_uri)?;
    let failed = results
        .iter()
        .filter(|result| result.error.is_some())
        .count();
    Ok((results.len(), failed))
}

/// Detailed, cancellable trash emptying (F08). Each entry is attempted
/// even when a previous one failed; a cancelled batch stops early.
pub fn empty_dir_detailed_cancellable(
    dir_uri: &str,
    cancellable: &gio::Cancellable,
) -> Result<Vec<DeleteResult>, glib::Error> {
    let dir = gio::File::for_uri(dir_uri);
    let children = dir.enumerate_children(
        "standard::name",
        gio::FileQueryInfoFlags::NONE,
        Some(cancellable),
    )?;
    let mut uris = Vec::new();
    while let Some(info) = children.next_file(Some(cancellable))? {
        uris.push(children.child(&info).uri().to_string());
    }
    drop(children);
    let mut results = Vec::new();
    for uri in uris {
        if cancellable.is_cancelled() {
            results.push(DeleteResult {
                uri,
                error: Some(cancelled_error()),
            });
            continue;
        }
        let error = delete_recursive_cancellable(&uri, cancellable).err();
        results.push(DeleteResult { uri, error });
    }
    Ok(results)
}

/// Empties a directory and retains each attempted URI and its technical
/// error for a caller that needs an operation report.
pub fn empty_dir_detailed(dir_uri: &str) -> Result<Vec<DeleteResult>, glib::Error> {
    let dir = gio::File::for_uri(dir_uri);
    let children = dir.enumerate_children(
        "standard::name",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    )?;
    // List first, then delete: deleting while iterating
    // over lazy-read backends hides the remaining entries.
    let mut uris = Vec::new();
    while let Some(info) = children.next_file(gio::Cancellable::NONE)? {
        uris.push(children.child(&info).uri().to_string());
    }
    drop(children);
    Ok(uris
        .into_iter()
        .map(|uri| {
            let error = delete_recursive(&uri).err();
            DeleteResult { uri, error }
        })
        .collect())
}

/// Empties the trash: all entries are deleted permanently.
pub fn empty_trash() -> Result<(usize, usize), glib::Error> {
    empty_dir(TRASH_URI)
}

/// Detailed results for emptying Trash after the UI has confirmed it.
pub fn empty_trash_detailed() -> Result<Vec<DeleteResult>, glib::Error> {
    empty_dir_detailed(TRASH_URI)
}

/// Cancellable trash restore (F08). Same semantics as [`restore`].
pub fn restore_cancellable(
    uri: &str,
    cancellable: &gio::Cancellable,
) -> Result<String, glib::Error> {
    let file = gio::File::for_uri(uri);
    let info = file.query_info(
        gio::FILE_ATTRIBUTE_TRASH_ORIG_PATH.as_str(),
        gio::FileQueryInfoFlags::NONE,
        Some(cancellable),
    )?;
    let dest = restore_destination_cancellable(&info, Some(cancellable))?;
    file.move_(
        &dest,
        gio::FileCopyFlags::NONE,
        Some(cancellable),
        Some(&mut |_, _| {}),
    )
    .map(|_| dest.uri().into())
}

/// Restores a trash entry to its original location.
/// The name comes from the original path (the trash basename may
/// differ); if taken, the existing entry is preserved with a suffix.
/// Returns the final URI. On error the entry stays in the trash.
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

/// Reads a GIO byte-string attribute as raw bytes, without going
/// through the gtk-rs getter (whose debug assert demands UTF-8 even if
/// the content is opaque: `trash::orig-path` is a byte string). Holds in
/// debug as in release.
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

/// Restore destination from trash metadata: original directory
/// + original name with suffix if taken. The original
///   path is a byte string (`trash::orig-path`, not
///   `standard::trash::orig-path`) and must be read as bytes to preserve
///   spaces, special chars and non-UTF-8 names. Testable without backend.
fn restore_destination(info: &gio::FileInfo) -> Result<gio::File, glib::Error> {
    restore_destination_cancellable(info, gio::Cancellable::NONE)
}

fn restore_destination_cancellable(
    info: &gio::FileInfo,
    cancellable: Option<&gio::Cancellable>,
) -> Result<gio::File, glib::Error> {
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
    let parent_file = gio::File::for_path(parent);
    let parent_info = parent_file.query_info(
        "standard::type",
        gio::FileQueryInfoFlags::NONE,
        cancellable,
    );
    let parent_info = match parent_info {
        Ok(info) => info,
        Err(error) if error.matches(gio::IOErrorEnum::NotFound) => {
            return Err(io_error("Original location is no longer available"));
        }
        Err(error) => return Err(error),
    };
    if parent_info.file_type() != gio::FileType::Directory {
        return Err(io_error("Original location is no longer available"));
    }
    match cancellable {
        Some(cancellable) => unique_child_cancellable(&parent_file, name, cancellable),
        None => Ok(unique_child(&parent_file, name)),
    }
}

/// Creates an empty file inside `parent_dir_uri`. If the name is taken,
/// adds a suffix (`name (copy).ext`). Returns the new URI.
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

/// Metadata for the "Properties" window.
#[derive(Debug, Clone)]
pub struct Props {
    pub name: String,
    pub uri: String,
    pub is_dir: bool,
    pub size: i64,
    /// Unix seconds, `None` if unknown.
    pub modified: Option<i64>,
    pub content_type: Option<String>,
}

/// Reads name, type, size, date and content of `uri`.
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

/// Renames. Returns the new URI.
pub fn rename(uri: &str, new_name: &str) -> Result<String, glib::Error> {
    let new_name = new_name.trim();
    if new_name.is_empty() || new_name.contains('/') {
        return Err(io_error("Invalid file name"));
    }
    let renamed = gio::File::for_uri(uri).set_display_name(new_name, gio::Cancellable::NONE)?;
    Ok(renamed.uri().into())
}

/// Creates the `name` folder inside `parent_dir_uri`. Returns the new URI.
/// Errors if the name is empty, contains `/`, is `.`/`..` or exists.
pub fn mkdir(parent_dir_uri: &str, name: &str) -> Result<String, glib::Error> {
    let name = name.trim();
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(io_error("Invalid folder name"));
    }
    let child = gio::File::for_uri(parent_dir_uri).child(name);
    child.make_directory(gio::Cancellable::NONE)?;
    Ok(child.uri().into())
}

/// Splits `name` into (stem, extension) at byte level: the last
/// non-leading `.`. Byte-safe version for non-UTF-8 names.
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

/// `dest_dir/name`, or `name (copy).ext`, `name (copy 2).ext`...
/// Works on `OsStr` to preserve non-UTF-8 names.
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

/// `true` if `dest_dir` is `src` or one of its descendants: copying a
/// directory there would create the destination before enumerating it,
/// and recursion would copy it forever (nested copies).
fn dest_inside_src(src: &gio::File, dest_dir: &gio::File) -> bool {
    // Local paths: physical identity via canonicalization (resolves
    // `..`, symlinks in the destination itself and in intermediate
    // components). No textual prefixes: `Path::starts_with` works
    // per component (`/tmp/A2` is not inside `/tmp/A`).
    if let (Some(src_path), Some(dest_path)) = (src.path(), dest_dir.path()) {
        if let (Ok(src_canon), Ok(dest_canon)) = (
            std::fs::canonicalize(&src_path),
            std::fs::canonicalize(&dest_path),
        ) {
            return dest_canon.starts_with(&src_canon);
        }
        // Source or destination not canonicalizable (missing): let
        // the copy fail with its natural error.
        return false;
    }
    // Non-local backends (trash://, network://, ...): no physical
    // identity nor symlinks to resolve; comparison on normalized URIs
    // with separator guard. Limitation: aliases of the same object with
    // different URIs (or different case on case-insensitive backends)
    // are not detected.
    let norm = |uri: glib::GString| {
        let s = uri.to_string();
        s.trim_end_matches('/').to_string()
    };
    let s = norm(src.uri());
    let d = norm(dest_dir.uri());
    d == s || d.starts_with(&format!("{s}/"))
}

/// Cancellable recursive copy used by the F08 job center: `cancellable` is
/// propagated to every GIO call, including the file-copy progress callback
/// (reported as measured bytes of the current file, never a fabricated
/// total). Keep in sync with [`copy_to`]: same guards and naming.
pub fn copy_to_cancellable(
    src_uri: &str,
    dest_dir_uri: &str,
    cancellable: &gio::Cancellable,
    mut progress: impl FnMut(i64, i64),
) -> Result<(), glib::Error> {
    let src = gio::File::for_uri(src_uri);
    let dest_dir = gio::File::for_uri(dest_dir_uri);
    if src.query_file_type(gio::FileQueryInfoFlags::NONE, Some(cancellable))
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
    let dest = unique_child_cancellable(
        &gio::File::for_uri(dest_dir_uri),
        OsStr::new(&name),
        cancellable,
    )?;
    copy_recursive_cancellable(&src, &dest, cancellable, &mut progress).map_err(|error| {
        copy_error_with_partial_target(error, &dest)
    })
}

fn copy_error_with_partial_target(error: glib::Error, dest: &gio::File) -> glib::Error {
    // A target already present on entry (or appearing due to a collision) is
    // not evidence that this job created it. For all other failures, query
    // without following symlinks so a broken link also counts as an output.
    let partial_exists = !error.matches(gio::IOErrorEnum::Exists)
        && dest
            .query_info(
                "standard::type",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                gio::Cancellable::NONE,
            )
            .is_ok();
    if !partial_exists {
        return error;
    }

    let message = format!(
        "{error}; partial copy output may remain at {}",
        dest.uri()
    );
    if error.matches(gio::IOErrorEnum::Cancelled) {
        glib::Error::new(gio::IOErrorEnum::Cancelled, &message)
    } else {
        glib::Error::new(gio::IOErrorEnum::Failed, &message)
    }
}

fn copy_recursive_cancellable(
    src: &gio::File,
    dest: &gio::File,
    cancellable: &gio::Cancellable,
    progress: &mut impl FnMut(i64, i64),
) -> Result<(), glib::Error> {
    if cancellable.is_cancelled() {
        return Err(cancelled_error());
    }
    if src.query_file_type(gio::FileQueryInfoFlags::NONE, Some(cancellable))
        == gio::FileType::Directory
    {
        dest.make_directory(Some(cancellable))?;
        let children = src.enumerate_children(
            "standard::name",
            gio::FileQueryInfoFlags::NONE,
            Some(cancellable),
        )?;
        while let Some(info) = children.next_file(Some(cancellable))? {
            copy_recursive_cancellable(
                &children.child(&info),
                &dest.child(info.name()),
                cancellable,
                progress,
            )?;
        }
        Ok(())
    } else {
        let total = src
            .query_info("standard::size", gio::FileQueryInfoFlags::NONE, Some(cancellable))
            .ok()
            .map(|info| info.size())
            .filter(|size| *size > 0)
            .unwrap_or(-1);
        let mut last = 0_i64;
        src.copy(
            dest,
            gio::FileCopyFlags::NONE,
            Some(cancellable),
            Some(&mut |current, _total| {
                // GIO reports measured bytes of the current file; throttle
                // to whole-percent steps to keep the UI callback cheap.
                let percent = if total > 0 { current * 100 / total } else { current };
                if percent != last {
                    last = percent;
                    progress(current, total);
                }
            }),
        )
        .map(|_| ())
    }
}

fn cancelled_error() -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::Cancelled, "Operation cancelled")
}

/// `unique_child` with a propagated cancellable for the existence probes.
fn unique_child_cancellable(
    dest_dir: &gio::File,
    name: &OsStr,
    cancellable: &gio::Cancellable,
) -> Result<gio::File, glib::Error> {
    let mut candidate = dest_dir.child(name);
    if !child_exists_cancellable(&candidate, cancellable)? {
        return Ok(candidate);
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
        if !child_exists_cancellable(&candidate, cancellable)? {
            return Ok(candidate);
        }
        i += 1;
    }
}

fn child_exists_cancellable(
    file: &gio::File,
    cancellable: &gio::Cancellable,
) -> Result<bool, glib::Error> {
    if cancellable.is_cancelled() {
        return Err(cancelled_error());
    }
    let result = file.query_info(
        "standard::type",
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(cancellable),
    );
    if cancellable.is_cancelled() {
        return Err(cancelled_error());
    }
    match result {
        Ok(_) => Ok(true),
        Err(error) if error.matches(gio::IOErrorEnum::NotFound) => Ok(false),
        Err(error) => Err(error),
    }
}

/// Copies a file or folder (recursively) into `dest_dir_uri`.
/// Copying a directory into itself or one of its descendants is
/// rejected before creating anything.
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

/// Moves into `dest_dir_uri`. May fail across filesystems:
/// the UI shows the error (copy+delete fallback coming).
pub fn move_to(src_uri: &str, dest_dir_uri: &str) -> Result<(), glib::Error> {
    let src = gio::File::for_uri(src_uri);
    let dest_dir = gio::File::for_uri(dest_dir_uri);
    if src.query_file_type(gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
        == gio::FileType::Directory
        && dest_inside_src(&src, &dest_dir)
    {
        return Err(io_error(
            "Cannot move a folder into itself or one of its subfolders",
        ));
    }
    if src.parent().is_some_and(|parent| parent.equal(&dest_dir)) {
        return Err(io_error("Source is already in the destination folder"));
    }
    let name = src
        .basename()
        .and_then(|n| n.into_string().ok())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| io_error("Invalid file name"))?;
    let dest = unique_child(&dest_dir, OsStr::new(&name));
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
    fn virtual_uri_listing_preserves_real_targets_and_missing_favorites() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        let file = tmp.path().join(".hidden-file");
        std::fs::write(&file, b"favorite").unwrap();
        let missing = tmp.path().join("missing");
        let folder_uri = gio::File::for_path(&folder).uri().to_string();
        let file_uri = gio::File::for_path(&file).uri().to_string();
        let missing_uri = gio::File::for_path(&missing).uri().to_string();
        let items = vec![
            ("Favorite folder".into(), folder_uri.clone()),
            ("Favorite file".into(), file_uri.clone()),
            ("Missing".into(), missing_uri.clone()),
        ];
        let (entries, _) =
            list_uri_entries(&items, SortOrder::default(), &gio::Cancellable::new()).unwrap();
        let folder_entry = entries.iter().find(|item| item.uri == folder_uri).unwrap();
        assert!(folder_entry.is_dir);
        assert_eq!(folder_entry.name, "folder");
        let file_entry = entries.iter().find(|item| item.uri == file_uri).unwrap();
        assert!(!file_entry.is_dir);
        assert_eq!(file_entry.size, 8);
        assert_eq!(file_entry.name, ".hidden-file");
        let missing_entry = entries.iter().find(|item| item.uri == missing_uri).unwrap();
        assert!(!missing_entry.is_dir);
        assert!(missing_entry.size < 0);
    }

    #[test]
    fn query_entry_returns_metadata_for_uri_with_spaces() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("directory with spaces");
        std::fs::create_dir(&path).unwrap();
        let file_path = path.join("report final.txt");
        std::fs::write(&file_path, b"report").unwrap();

        let uri = gio::File::for_path(&file_path).uri();
        let entry = query_entry(uri.as_ref()).unwrap();
        assert_eq!(entry.name, "report final.txt");
        assert_eq!(entry.size, 6);
        assert!(!entry.is_dir);
        assert!(entry.uri.contains("%20") || entry.uri.contains(' '));
    }

    #[test]
    fn list_dir_orders_dirs_first() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("zebra-dir")).unwrap();
        std::fs::create_dir(tmp.path().join("alpha-dir")).unwrap();
        std::fs::write(tmp.path().join("file.txt"), b"ciao").unwrap();

        let uri = format!("file://{}", tmp.path().display());
        let entries = list_dir(&uri, true).unwrap();

        assert_eq!(entries.len(), 3);
        // Directories first (sorted), then files.
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

    #[test]
    fn list_dir_honors_a_pre_cancelled_gio_cancellable() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("entry.txt"), b"x").unwrap();
        let cancellable = gio::Cancellable::new();
        cancellable.cancel();

        let uri = gio::File::for_path(tmp.path()).uri().to_string();
        assert!(list_dir_with_cancellable(&uri, true, &cancellable).is_err());
    }

    #[test]
    fn detailed_empty_dir_preserves_item_uris_and_successes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), b"a").unwrap();
        std::fs::write(tmp.path().join("b.txt"), b"b").unwrap();
        let uri = gio::File::for_path(tmp.path()).uri().to_string();

        let mut results = empty_dir_detailed(&uri).unwrap();
        results.sort_by(|left, right| left.uri.cmp(&right.uri));
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|result| result.error.is_none()));
        assert!(results[0].uri.contains("a.txt"));
        assert!(results[1].uri.contains("b.txt"));
    }

    #[test]
    fn list_dir_icons_survive_string_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("docs")).unwrap();
        std::fs::write(tmp.path().join("note.txt"), b"hello").unwrap();
        let uri = format!("file://{}", tmp.path().display());
        for entry in list_dir(&uri, true).unwrap() {
            let icon_str = entry.icon.expect("enumerator provides an icon");
            assert!(!icon_str.is_empty(), "{}", entry.name);
            // The worker stores strings; the view parses them back.
            assert!(
                gio::Icon::for_string(&icon_str).is_ok(),
                "{}: {icon_str}",
                entry.name
            );
        }
    }

    #[test]
    fn list_dir_sorts_case_insensitive_with_dirs_first() {
        let tmp = tempfile::tempdir().unwrap();
        for name in [
            "zebra-dir",
            "Alpha-dir",
            "BETA",
            "beta",
            "café ☃",
            "CAFÉ",
            "100%",
            "a#b",
        ] {
            if name.ends_with("-dir") {
                std::fs::create_dir(tmp.path().join(name)).unwrap();
            } else {
                std::fs::write(tmp.path().join(name), b"x").unwrap();
            }
        }
        let uri = format!("file://{}", tmp.path().display());
        let names: Vec<(String, bool)> = list_dir(&uri, true)
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.is_dir))
            .collect();
        // Directories first, then case-insensitive name order.
        let dirs: Vec<&str> = names
            .iter()
            .filter(|(_, d)| *d)
            .map(|(n, _)| n.as_str())
            .collect();
        assert_eq!(dirs, vec!["Alpha-dir", "zebra-dir"]);
        let files: Vec<&str> = names
            .iter()
            .filter(|(_, d)| !*d)
            .map(|(n, _)| n.as_str())
            .collect();
        let mut sorted = files.clone();
        sorted.sort_by_key(|n| n.to_lowercase());
        assert_eq!(files, sorted);
        // Equal keys keep enumeration order (stable): BETA before beta.
        let beta: Vec<&&str> = files
            .iter()
            .filter(|n| n.to_lowercase() == "beta")
            .collect();
        assert_eq!(beta, vec![&"BETA", &"beta"]);
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
    fn cancelled_identity_query_returns_cancelled_without_touching_source() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source.txt");
        std::fs::write(&source, b"untouched").unwrap();
        let cancellable = gio::Cancellable::new();
        cancellable.cancel();

        let error = file_identity_cancellable(&uri(&source), &cancellable).unwrap_err();
        assert!(error.matches(gio::IOErrorEnum::Cancelled));
        assert_eq!(std::fs::read(source).unwrap(), b"untouched");
    }

    #[test]
    fn pre_cancelled_trash_does_not_touch_source_or_session_trash() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source.txt");
        std::fs::write(&source, b"untouched").unwrap();
        let cancellable = gio::Cancellable::new();
        cancellable.cancel();

        let error = trash_with_identity_cancellable(&uri(&source), &cancellable).unwrap_err();
        assert!(error.matches(gio::IOErrorEnum::Cancelled));
        assert_eq!(std::fs::read(source).unwrap(), b"untouched");
    }

    #[test]
    fn cancelled_followup_metadata_after_trash_remains_successful() {
        let result = trash_metadata_after_success(Err(glib::Error::new(
            gio::IOErrorEnum::Cancelled,
            "metadata query cancelled after Trash succeeded",
        )));
        assert!(result.is_none());
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
    fn cancellable_child_collision_gets_copy_suffix() {
        let tmp = tempfile::tempdir().unwrap();
        let existing = tmp.path().join("item.txt");
        std::fs::write(&existing, b"existing").unwrap();

        let candidate = unique_child_cancellable(
            &gio::File::for_path(tmp.path()),
            OsStr::new("item.txt"),
            &gio::Cancellable::new(),
        )
        .unwrap();

        assert!(candidate.equal(&gio::File::for_path(tmp.path().join("item (copy).txt"))));
        assert_eq!(std::fs::read(existing).unwrap(), b"existing");
    }

    #[test]
    fn mkdir_creates_folder_and_validates_name() {
        let tmp = tempfile::tempdir().unwrap();
        let parent = uri(tmp.path());

        let made = mkdir(&parent, "Nuova cartella").unwrap();
        assert!(tmp.path().join("Nuova cartella").is_dir());
        assert!(made.ends_with("Nuova%20cartella") || made.ends_with("Nuova cartella"));

        // Existing, empty, with slash, `.` and `..`: all errors.
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

        // Recursive: the full folder and the file go away together.
        let (total, failed) = empty_dir(&uri(tmp.path())).unwrap();
        assert_eq!((total, failed), (2, 0));
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
        // Already empty directory: zero entries, no error.
        assert_eq!(empty_dir(&uri(tmp.path())).unwrap(), (0, 0));
    }

    #[test]
    fn restore_requires_a_trash_entry() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("file.txt"), b"x").unwrap();

        // Plain file: without `trash::orig-path` it cannot be restored.
        assert!(restore(&uri(&tmp.path().join("file.txt"))).is_err());
        assert!(tmp.path().join("file.txt").exists());
    }

    /// FileInfo with only the attribute that matters, as the backend gives it.
    fn trash_info(orig_path: &str) -> gio::FileInfo {
        let info = gio::FileInfo::new();
        info.set_attribute_byte_string(gio::FILE_ATTRIBUTE_TRASH_ORIG_PATH.as_str(), orig_path);
        info
    }

    #[test]
    fn restore_destination_reads_orig_path() {
        let tmp = tempfile::tempdir().unwrap();
        let orig = tmp.path().join("my doc (1) [x].txt");

        // Name from the original path, spaces and specials intact.
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
        // The existing entry is untouched: only the free name is computed.
        assert_eq!(std::fs::read(&orig).unwrap(), b"existing");
    }

    #[test]
    fn restore_destination_missing_metadata_errors() {
        // No attributes.
        let err = restore_destination(&gio::FileInfo::new()).unwrap_err();
        assert!(err.to_string().contains("Unknown original location"));

        // The old wrong attribute name is no longer read.
        let info = gio::FileInfo::new();
        info.set_attribute_byte_string("standard::trash::orig-path", "/tmp/x.txt");
        let err = restore_destination(&info).unwrap_err();
        assert!(err.to_string().contains("Unknown original location"));

        // Original directory gone.
        let missing = trash_info("/tmp/kito-definitely-gone-xyz/f.txt");
        let err = restore_destination(&missing).unwrap_err();
        assert!(err.to_string().contains("no longer available"));
    }

    #[test]
    fn restore_destination_resolution_honors_pre_cancelled_cancellable() {
        let tmp = tempfile::tempdir().unwrap();
        let fixture = tmp.path().join("report.txt");
        std::fs::write(&fixture, b"keep intact").unwrap();
        let info = trash_info(fixture.to_str().unwrap());
        let cancellable = gio::Cancellable::new();
        cancellable.cancel();

        let error = restore_destination_cancellable(&info, Some(&cancellable)).unwrap_err();
        assert!(error.matches(gio::IOErrorEnum::Cancelled));
        assert_eq!(std::fs::read(&fixture).unwrap(), b"keep intact");
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

    /// Trashes `path` via GIO. Returns `None` (skip) if the backend is
    /// unavailable in this environment; never touches other entries.
    fn trash_tempfile(path: &std::path::Path) -> Option<()> {
        if let Err(e) = gio::File::for_path(path).trash(gio::Cancellable::NONE) {
            eprintln!("SKIP trash e2e: backend unavailable ({e})");
            return None;
        }
        assert!(!path.exists());
        Some(())
    }

    /// `trash:///` URI of the entry whose origin is `orig`, or panic.
    /// Compares raw bytes (no UTF-8 conversions: other trash entries
    /// may have arbitrary names). gvfsd notices new entries via a
    /// monitor: brief retry before giving up.
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

    /// Temp dir under home: /tmp lives on a mount where trash is not
    /// supported, home is. Only test-owned entries, never existing ones.
    fn home_tempdir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("kito-restore-test-")
            .tempdir_in(glib::home_dir().join(".cache"))
            .unwrap()
    }

    #[test]
    fn restore_roundtrip_through_trash() {
        // Unique entry name per process: reusing the same name in close
        // runs confuses the gvfsd-trash monitor (coalesced events, the
        // entry then never shows up).
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

        // Single trash->restore cycle: rapid repeated cycles on the
        // same name lose events in the gvfsd-trash monitor (its cache
        // diverges and the entry never shows up; daemon problem, not a
        // restore one). Collisions are covered at unit level.
        let restored = restore(&find_trash_entry(&path)).unwrap();
        assert_eq!(restored, gio::File::for_path(&path).uri().to_string());
        assert_eq!(std::fs::read(&path).unwrap(), b"payload");
    }

    /// Sets `trash::orig-path` to raw bytes via direct C API (the
    /// binding setter only accepts `&str`). Tests only.
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
    fn checked_move_inverse_restores_only_matching_identity_and_never_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let origin = tmp.path().join("origin");
        let moved = tmp.path().join("moved");
        let other = tmp.path().join("other");
        std::fs::create_dir(&origin).unwrap();
        std::fs::create_dir(&moved).unwrap();
        std::fs::write(origin.join("item"), b"payload").unwrap();
        let source = gio::File::for_path(origin.join("item")).uri().to_string();
        let target = gio::File::for_path(moved.join("item")).uri().to_string();
        let identity = file_identity(&source).unwrap();
        move_to_exact_result(&source, gio::File::for_path(&moved).uri().as_ref()).unwrap();
        move_exact_checked(&target, &source, &identity).unwrap();
        assert_eq!(std::fs::read(origin.join("item")).unwrap(), b"payload");

        let new_target =
            move_to_exact_result(&source, gio::File::for_path(&moved).uri().as_ref()).unwrap();
        std::fs::write(origin.join("item"), b"keep existing").unwrap();
        assert!(move_exact_checked(&new_target, &source, &identity).is_err());
        assert_eq!(
            std::fs::read(origin.join("item")).unwrap(),
            b"keep existing"
        );
        assert_eq!(std::fs::read(moved.join("item")).unwrap(), b"payload");

        let wrong = FileIdentity {
            filesystem: "wrong filesystem".into(),
            file: "wrong file".into(),
        };
        assert!(move_exact_checked(
            &new_target,
            gio::File::for_path(other.join("item")).uri().as_ref(),
            &wrong
        )
        .is_err());
        assert_eq!(std::fs::read(moved.join("item")).unwrap(), b"payload");
    }

    #[test]
    fn strict_restore_destination_conflict_check_does_not_choose_suffix() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("present");
        std::fs::write(&dest, b"untouched").unwrap();
        let uri = gio::File::for_path(&dest).uri().to_string();
        let expected = FileIdentity {
            filesystem: "x".into(),
            file: "y".into(),
        };
        assert!(restore_exact_checked(&uri, &uri, &expected).is_err());
        assert_eq!(std::fs::read(dest).unwrap(), b"untouched");
    }

    #[test]
    fn delete_symlink_to_external_dir_keeps_target() {
        let external = tempfile::tempdir().unwrap();
        std::fs::write(external.path().join("keep.txt"), b"do not touch").unwrap();
        let home = tempfile::tempdir().unwrap();
        let link = home.path().join("link");
        std::os::unix::fs::symlink(external.path(), &link).unwrap();

        delete_recursive(&uri(&link)).unwrap();

        // Only the link disappears: target intact.
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
        // Link to the directory containing it: following it
        // would recurse forever.
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
        // File symlink inside the tree: goes away with the rest.
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

    #[test]
    fn cancellable_delete_propagates_missing_directory_enumeration_error() {
        let tmp = tempfile::tempdir().unwrap();
        let error = delete_recursive_cancellable(
            &uri(&tmp.path().join("missing-directory")),
            &gio::Cancellable::new(),
        )
        .unwrap_err();
        assert!(error.matches(gio::IOErrorEnum::NotFound));
    }

    /// Sorted entry names of `dir`: snapshot to verify that a rejected
    /// operation creates nothing.
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
    fn cancellable_copy_missing_source_keeps_original_retryable_error() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("missing.txt");
        let error = copy_to_cancellable(
            &uri(&missing),
            &uri(tmp.path()),
            &gio::Cancellable::new(),
            |_, _| {},
        )
        .unwrap_err();

        assert!(error.matches(gio::IOErrorEnum::NotFound));
        assert!(!error.message().contains("partial copy output may remain at"));
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn partial_copy_marker_requires_a_present_non_collision_target() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("partial.txt");
        std::fs::write(&target, b"partial").unwrap();
        let dest = gio::File::for_path(&target);
        let error = glib::Error::new(gio::IOErrorEnum::Cancelled, "cancelled");

        let marked = copy_error_with_partial_target(error, &dest);
        assert!(marked.matches(gio::IOErrorEnum::Cancelled));
        assert!(marked.message().contains("partial copy output may remain at"));
        assert!(marked.message().contains(dest.uri().as_str()));

        let collision = copy_error_with_partial_target(
            glib::Error::new(gio::IOErrorEnum::Exists, "already exists"),
            &dest,
        );
        assert!(collision.matches(gio::IOErrorEnum::Exists));
        assert!(!collision.message().contains("partial copy output may remain at"));
    }

    #[test]
    fn partial_move_marker_requires_a_present_non_collision_target() {
        let tmp = tempfile::tempdir().unwrap();
        let target_path = tmp.path().join("partial.txt");
        let source_path = tmp.path().join("source.txt");
        std::fs::write(&source_path, b"source").unwrap();
        let dest = gio::File::for_path(&target_path);

        let absent = move_error_with_partial_target(
            glib::Error::new(gio::IOErrorEnum::NotFound, "source missing"),
            &dest,
        );
        assert!(absent.matches(gio::IOErrorEnum::NotFound));
        assert!(!absent.message().contains("partial move output may remain at"));

        std::fs::write(&target_path, b"partial").unwrap();
        let partial = move_error_with_partial_target(
            glib::Error::new(gio::IOErrorEnum::Cancelled, "cancelled"),
            &dest,
        );
        assert!(partial.matches(gio::IOErrorEnum::Cancelled));
        assert!(partial.message().contains("partial move output may remain at"));
        assert!(partial.message().contains(dest.uri().as_str()));
        assert_eq!(std::fs::read(&source_path).unwrap(), b"source");

        let collision = move_error_with_partial_target(
            glib::Error::new(gio::IOErrorEnum::Exists, "already exists"),
            &dest,
        );
        assert!(collision.matches(gio::IOErrorEnum::Exists));
        assert!(!collision.message().contains("partial move output may remain at"));
        assert_eq!(std::fs::read(&source_path).unwrap(), b"source");
        assert!(target_path.exists(), "the helper must not clean up the target");
    }

    #[test]
    fn cancellable_copy_preserves_cancellation_without_partial_output() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source.txt");
        let dest = tmp.path().join("dest");
        std::fs::write(&source, b"source").unwrap();
        std::fs::create_dir(&dest).unwrap();
        let cancellable = gio::Cancellable::new();
        cancellable.cancel();

        let error = copy_to_cancellable(
            &uri(&source),
            &uri(&dest),
            &cancellable,
            |_, _| {},
        )
        .unwrap_err();

        assert!(error.matches(gio::IOErrorEnum::Cancelled));
        assert!(!error.message().contains("partial copy output may remain at"));
        assert_eq!(std::fs::read_dir(dest).unwrap().count(), 0);
    }

    #[test]
    fn pre_cancelled_copy_with_existing_basename_is_not_partial_output() {
        let tmp = tempfile::tempdir().unwrap();
        let source_dir = tmp.path().join("source");
        let dest_dir = tmp.path().join("dest");
        std::fs::create_dir(&source_dir).unwrap();
        std::fs::create_dir(&dest_dir).unwrap();
        let source = source_dir.join("item.txt");
        let target = dest_dir.join("item.txt");
        std::fs::write(&source, b"source").unwrap();
        std::fs::write(&target, b"existing target").unwrap();
        let cancellable = gio::Cancellable::new();
        cancellable.cancel();

        let error = copy_to_cancellable(
            &uri(&source),
            &uri(&dest_dir),
            &cancellable,
            |_, _| {},
        )
        .unwrap_err();

        assert!(error.matches(gio::IOErrorEnum::Cancelled));
        assert!(!error.message().contains("partial copy output may remain at"));
        assert_eq!(std::fs::read(source).unwrap(), b"source");
        assert_eq!(std::fs::read(target).unwrap(), b"existing target");
        assert_eq!(std::fs::read_dir(dest_dir).unwrap().count(), 1);
    }

    #[test]
    fn pre_cancelled_move_with_existing_basename_is_not_partial_output() {
        let tmp = tempfile::tempdir().unwrap();
        let source_dir = tmp.path().join("source");
        let dest_dir = tmp.path().join("dest");
        std::fs::create_dir(&source_dir).unwrap();
        std::fs::create_dir(&dest_dir).unwrap();
        let source = source_dir.join("item.txt");
        let target = dest_dir.join("item.txt");
        std::fs::write(&source, b"source").unwrap();
        std::fs::write(&target, b"existing target").unwrap();
        let cancellable = gio::Cancellable::new();
        cancellable.cancel();

        let error = move_to_exact_result_cancellable(
            &uri(&source),
            &uri(&dest_dir),
            &cancellable,
        )
        .unwrap_err();

        assert!(error.matches(gio::IOErrorEnum::Cancelled));
        assert!(!error.message().contains("partial move output may remain at"));
        assert_eq!(std::fs::read(source).unwrap(), b"source");
        assert_eq!(std::fs::read(target).unwrap(), b"existing target");
        assert_eq!(std::fs::read_dir(dest_dir).unwrap().count(), 1);
    }

    #[test]
    fn copy_dir_into_itself_is_rejected() {
        let tmp = copy_fixture();
        let a = tmp.path().join("A");
        let before = names(&a);

        assert!(copy_to(&uri(&a), &uri(&a)).is_err());

        // Nothing created, source intact.
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

        // Destination reaching A or one of its descendants via symlink.
        assert!(copy_to(&uri(&a), &uri(&tmp.path().join("link_a"))).is_err());
        assert!(copy_to(&uri(&a), &uri(&tmp.path().join("link_sub"))).is_err());

        assert_eq!(names(&a), before);
        assert_eq!(std::fs::read(a.join("f.txt")).unwrap(), b"effe");
    }

    #[test]
    fn move_dir_into_self_or_descendant_is_rejected_without_changes() {
        let tmp = copy_fixture();
        let a = tmp.path().join("A");
        let before = names(&a);

        assert!(move_to(&uri(&a), &uri(&a)).is_err());
        assert!(move_to(&uri(&a), &uri(&a.join("sub"))).is_err());

        assert_eq!(names(&a), before);
        assert_eq!(std::fs::read(a.join("f.txt")).unwrap(), b"effe");
        assert_eq!(std::fs::read(a.join("sub").join("g.txt")).unwrap(), b"gi");
    }

    #[test]
    fn move_dir_through_symlink_destination_is_rejected() {
        let tmp = copy_fixture();
        let a = tmp.path().join("A");
        std::os::unix::fs::symlink(a.join("sub"), tmp.path().join("link_sub")).unwrap();

        assert!(move_to(&uri(&a), &uri(&tmp.path().join("link_sub"))).is_err());
        assert!(a.join("f.txt").exists());
        assert!(a.join("sub").join("g.txt").exists());
    }

    #[test]
    fn move_to_the_same_parent_is_rejected_instead_of_renaming_as_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("report.txt");
        std::fs::write(&file, b"data").unwrap();

        assert!(move_to(&uri(&file), &uri(tmp.path())).is_err());
        assert_eq!(std::fs::read(file).unwrap(), b"data");
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
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

        // `A2` is not inside `A` despite the prefix: valid copies.
        copy_to(&uri(&a2), &uri(&a)).unwrap();
        assert_eq!(std::fs::read(a.join("A2").join("b.txt")).unwrap(), b"b");
        copy_to(&uri(&a), &uri(&a2)).unwrap();
        assert_eq!(std::fs::read(a2.join("A").join("a.txt")).unwrap(), b"a");
        // Sources intact.
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

        // Same name: suffix, no overwrite.
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

    fn special_dirs() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        for name in [
            "My Folder",
            "café ☃",
            "100%",
            "a#b",
            "%20",
            "mix %20 # % ünï",
        ] {
            std::fs::create_dir(tmp.path().join(name)).unwrap();
        }
        tmp
    }

    #[test]
    fn uri_display_decodes_special_names() {
        let tmp = special_dirs();
        for name in [
            "My Folder",
            "café ☃",
            "100%",
            "a#b",
            "%20",
            "mix %20 # % ünï",
        ] {
            let path = tmp.path().join(name);
            let file_uri = gio::File::for_path(&path).uri().to_string();
            // Readable text, never the raw %XX form...
            assert_eq!(uri_to_display(&file_uri), path.to_string_lossy());
            // ...except non-local URIs pass through untouched.
            assert_eq!(uri_to_display(TRASH_URI), TRASH_URI);
            // Round-trip back to the same location.
            assert_eq!(uri_to_path(&file_uri).unwrap(), path);
            assert_eq!(uri_file_name(&file_uri).unwrap(), name);
        }
    }

    #[test]
    fn uri_to_path_rejects_non_local() {
        assert!(uri_to_path(TRASH_URI).is_none());
        assert!(uri_to_path("network:///").is_none());
        assert!(uri_to_path("smb://server/share").is_none());
    }

    #[test]
    fn uri_display_non_utf8_is_lossy_but_round_trips() {
        use std::os::unix::ffi::OsStrExt;
        let tmp = tempfile::tempdir().unwrap();
        let raw: Vec<u8> = {
            let mut v = tmp.path().as_os_str().as_bytes().to_vec();
            v.extend_from_slice(b"/bad \xff name");
            v
        };
        let path = std::path::PathBuf::from(OsStr::from_bytes(&raw));
        std::fs::create_dir(&path).unwrap();
        let file_uri = gio::File::for_path(&path).uri().to_string();

        // gio decodes the bytes; display is lossy...
        assert_eq!(uri_to_path(&file_uri).unwrap().as_os_str().as_bytes(), &raw);
        let shown = uri_to_display(&file_uri);
        assert!(shown.contains('\u{FFFD}'));
        // ...but Enter unchanged resolves to the exact stored URI.
        assert_eq!(
            resolve_path_text(&shown, &shown, &file_uri).unwrap(),
            file_uri
        );
    }

    #[test]
    fn resolve_path_text_round_trip_and_edits() {
        let tmp = special_dirs();
        // Unchanged text (Ctrl+L, Enter): same directory, no re-encoding.
        for name in ["My Folder", "100%", "a#b", "%20"] {
            let path = tmp.path().join(name);
            let file_uri = gio::File::for_path(&path).uri().to_string();
            let shown = uri_to_display(&file_uri);
            assert_eq!(
                resolve_path_text(&shown, &shown, &file_uri).unwrap(),
                file_uri
            );
        }
        // Typed plain path.
        let typed = tmp.path().join("My Folder").to_string_lossy().into_owned();
        assert_eq!(
            resolve_path_text(&typed, "/elsewhere", "file:///elsewhere").unwrap(),
            gio::File::for_path(&typed).uri().to_string()
        );
        // Typed URI, including non-local ones.
        assert_eq!(
            resolve_path_text(TRASH_URI, "/x", "file:///x").unwrap(),
            TRASH_URI
        );
        // Empty text navigates nowhere.
        assert!(resolve_path_text("   ", "/x", "file:///x").is_none());
    }
}
