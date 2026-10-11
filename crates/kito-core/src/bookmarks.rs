//! Freedesktop-style bookmarks (`~/.config/gtk-3.0/bookmarks`, lines
//! `uri name...`). Same file as Nautilus: pins are shared.

use gio::prelude::*;
use std::{collections::BTreeMap, io, path::PathBuf};

#[cfg(target_os = "linux")]
use std::{
    ffi::{c_char, c_void, CString},
    os::unix::ffi::OsStrExt,
};

fn default_name(uri: &str) -> String {
    crate::uri_file_name(uri)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| gio::File::for_uri(uri).parse_name().to_string())
}

pub fn path() -> PathBuf {
    glib::user_config_dir().join("gtk-3.0/bookmarks")
}

/// Normalizes a bookmark URI using the same trailing-slash rules as the
/// Freedesktop bookmarks file. URI roots (for example `file:///`) are kept.
pub fn normalize_uri(uri: &str) -> String {
    if uri.ends_with(":///") {
        uri.to_string()
    } else {
        uri.trim_end_matches('/').to_string()
    }
}

fn normalize(uri: &str) -> String {
    normalize_uri(uri)
}

pub fn read_from(path: &std::path::Path) -> Vec<(String, String)> {
    try_read_from(path).unwrap_or_default()
}

fn read_text(path: &std::path::Path) -> io::Result<String> {
    read_snapshot(path)?.map_or_else(
        || Ok(String::new()),
        |bytes| {
            String::from_utf8(bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        },
    )
}

/// Reads the exact file contents, distinguishing a missing file from an empty one.
fn read_snapshot(path: &std::path::Path) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Reads the current destination without following a final-component symlink.
/// This is used just before rename so the compare-and-replace path never writes
/// through a symlink if one appeared while the temporary file was being staged.
enum CommitSnapshot {
    Stable(Option<Vec<u8>>),
    Changed,
}

fn metadata_without_symlink(path: &std::path::Path) -> io::Result<Option<std::fs::Metadata>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bookmarks path is a symbolic link",
        )),
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn same_metadata_version(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev()
            && left.ino() == right.ino()
            && left.mode() == right.mode()
            && left.uid() == right.uid()
            && left.gid() == right.gid()
            && left.len() == right.len()
            && left.mtime() == right.mtime()
            && left.mtime_nsec() == right.mtime_nsec()
            && left.ctime() == right.ctime()
            && left.ctime_nsec() == right.ctime_nsec()
    }
    #[cfg(not(unix))]
    {
        left.len() == right.len()
            && left.modified().ok() == right.modified().ok()
            && left.permissions().readonly() == right.permissions().readonly()
    }
}

fn same_metadata_state(
    left: Option<&std::fs::Metadata>,
    right: Option<&std::fs::Metadata>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => same_metadata_version(left, right),
        _ => false,
    }
}

fn read_snapshot_for_commit(path: &std::path::Path) -> io::Result<CommitSnapshot> {
    // Read twice and compare metadata across both reads. This catches writes and
    // atomic replacements racing with the check, including a concurrent create
    // when our expected state was missing. It still cannot lock out writers
    // after the final stat and before rename.
    let before = metadata_without_symlink(path)?;
    let first_contents = read_snapshot(path)?;
    let between = metadata_without_symlink(path)?;
    let contents = read_snapshot(path)?;
    let after = metadata_without_symlink(path)?;

    if !same_metadata_state(before.as_ref(), between.as_ref())
        || !same_metadata_state(between.as_ref(), after.as_ref())
        || first_contents != contents
        || contents.is_some() != after.is_some()
    {
        Ok(CommitSnapshot::Changed)
    } else {
        Ok(CommitSnapshot::Stable(contents))
    }
}

fn parse(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                return None;
            }
            let mut parts = line.splitn(2, ' ');
            let uri = parts.next()?.trim();
            if uri.is_empty() {
                return None;
            }
            let name = parts
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| default_name(uri));
            Some((name, uri.to_string()))
        })
        .collect()
}

pub fn try_read_from(path: &std::path::Path) -> Result<Vec<(String, String)>, glib::Error> {
    read_text(path)
        .map(|text| parse(&text))
        .map_err(|error| glib::Error::new(gio::IOErrorEnum::Failed, &error.to_string()))
}

fn uri_in_line(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    trimmed.split_whitespace().next()
}

// Atomically writes `text` only if the current bytes still match `expected`.
//
// GIO's `replace_contents()` supports ETag preconditions, but the local backend
// checks the ETag while opening the replacement stream and renames the staged
// file later; it is not an atomic compare-and-swap with that rename. Linux also
// has no general rename-if-destination-is-this-version primitive. Comparing
// exact bytes and metadata immediately before rename therefore detects and
// retries changes already committed by another writer, but a non-cooperating
// writer can still change the file after the final comparison and before rename
// (or commit its own stale replacement afterward). This is best-effort conflict
// detection, not a CAS guarantee. GIO's local replace path can also fall back to
// in-place rewriting for hard-linked files unless replacement-destination
// semantics are requested, which would change the existing symlink/permission
// behavior and atomicity relied on here.

const STAGED_FILE_NAME: &str = "contents";

/// A 0700 directory keeps even a crash-orphaned staged file unreachable to
/// other users while original, potentially permissive ACLs and xattrs are copied.
struct PrivateStagingDir {
    path: PathBuf,
}

impl PrivateStagingDir {
    fn file_path(&self) -> PathBuf {
        self.path.join(STAGED_FILE_NAME)
    }
}

impl Drop for PrivateStagingDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.file_path());
        let _ = std::fs::remove_dir(&self.path);
    }
}

fn create_private_staging_dir(parent: &std::path::Path) -> io::Result<PrivateStagingDir> {
    #[cfg(unix)]
    {
        use std::{
            io::Read,
            os::unix::fs::{DirBuilderExt, PermissionsExt},
        };

        let mut entropy = std::fs::File::open("/dev/urandom")?;
        for _ in 0..32 {
            let mut random = [0u8; 16];
            entropy.read_exact(&mut random)?;
            let mut suffix = String::with_capacity(random.len() * 2);
            for byte in random {
                use std::fmt::Write as _;
                write!(&mut suffix, "{byte:02x}").expect("writing to a String cannot fail");
            }

            let path = parent.join(format!(".kito-bookmarks-{suffix}.tmp"));
            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(&path) {
                Ok(()) => {
                    let staging = PrivateStagingDir { path };
                    // Apply the restrictive mode explicitly as well as at mkdir
                    // time (the umask may only make it more restrictive).
                    std::fs::set_permissions(
                        &staging.path,
                        std::fs::Permissions::from_mode(0o700),
                    )?;
                    return Ok(staging);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }

        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "private bookmark staging directory collision",
        ))
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "private bookmark staging directories require Unix permissions",
        ))
    }
}

fn create_staging_file(staging: &PrivateStagingDir) -> io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        let file = options.open(staging.file_path())?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        let _ = staging;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "private bookmark staging files require Unix permissions",
        ))
    }
}

#[cfg(target_os = "linux")]
type ExtendedAttributes = BTreeMap<Vec<u8>, Vec<u8>>;

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn llistxattr(path: *const c_char, list: *mut c_char, size: usize) -> isize;
    fn lgetxattr(
        path: *const c_char,
        name: *const c_char,
        value: *mut c_void,
        size: usize,
    ) -> isize;
}

#[cfg(target_os = "linux")]
fn linux_xattr_names(path: &std::path::Path) -> io::Result<Vec<Vec<u8>>> {
    use std::ptr;

    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in bookmark path"))?;
    let size = unsafe { llistxattr(path.as_ptr(), ptr::null_mut(), 0) };
    if size < 0 {
        let error = io::Error::last_os_error();
        // A filesystem which explicitly does not implement xattrs cannot have
        // xattr-backed ACLs to preserve. Other errors are not safe to ignore.
        return if error.kind() == io::ErrorKind::Unsupported {
            Ok(Vec::new())
        } else {
            Err(error)
        };
    }
    if size == 0 {
        return Ok(Vec::new());
    }

    let mut names = Vec::new();
    names
        .try_reserve_exact(size as usize)
        .map_err(|_| io::Error::other("cannot allocate xattr name buffer"))?;
    names.resize(size as usize, 0);
    let actual_size = unsafe { llistxattr(path.as_ptr(), names.as_mut_ptr().cast(), names.len()) };
    if actual_size < 0 {
        return Err(io::Error::last_os_error());
    }
    if actual_size as usize != names.len() || names.last() != Some(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bookmark xattrs changed while listing them",
        ));
    }

    Ok(names
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .map(|name| name.to_vec())
        .collect())
}

#[cfg(target_os = "linux")]
fn linux_xattr_value(path: &std::path::Path, name: &[u8]) -> io::Result<Vec<u8>> {
    use std::ptr;

    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in bookmark path"))?;
    let name = CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "NUL in xattr name"))?;
    let size = unsafe { lgetxattr(path.as_ptr(), name.as_ptr(), ptr::null_mut(), 0) };
    if size < 0 {
        return Err(io::Error::last_os_error());
    }

    let mut value = Vec::new();
    value
        .try_reserve_exact(size as usize)
        .map_err(|_| io::Error::other("cannot allocate xattr value buffer"))?;
    value.resize(size as usize, 0);
    let value_ptr = if value.is_empty() {
        ptr::null_mut()
    } else {
        value.as_mut_ptr().cast()
    };
    let actual_size = unsafe { lgetxattr(path.as_ptr(), name.as_ptr(), value_ptr, value.len()) };
    if actual_size < 0 {
        return Err(io::Error::last_os_error());
    }
    if actual_size as usize != value.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bookmark xattr changed while reading it",
        ));
    }
    Ok(value)
}

#[cfg(target_os = "linux")]
fn snapshot_extended_attributes(
    path: &std::path::Path,
) -> io::Result<(std::fs::Metadata, ExtendedAttributes)> {
    for _ in 0..3 {
        let before = metadata_without_symlink(path)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "bookmark file disappeared"))?;
        let names = linux_xattr_names(path)?;
        let mut attributes = BTreeMap::new();
        for name in &names {
            attributes.insert(name.clone(), linux_xattr_value(path, name)?);
        }
        if linux_xattr_names(path)? != names {
            continue;
        }
        let after = metadata_without_symlink(path)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "bookmark file disappeared"))?;
        if same_metadata_version(&before, &after) {
            return Ok((after, attributes));
        }
    }

    Err(io::Error::other(
        "bookmark xattrs changed repeatedly while being checked",
    ))
}

#[cfg(not(target_os = "linux"))]
type ExtendedAttributes = BTreeMap<Vec<u8>, Vec<u8>>;

#[cfg(not(target_os = "linux"))]
fn snapshot_extended_attributes(
    _path: &std::path::Path,
) -> io::Result<(std::fs::Metadata, ExtendedAttributes)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "cannot verify ACLs and extended attributes on this platform",
    ))
}

#[derive(Debug, Eq, PartialEq)]
enum CopyAttributeValue {
    String(String),
    ByteString(Vec<u8>),
    Boolean(bool),
    Uint32(u32),
    Int32(i32),
    Uint64(u64),
    Int64(i64),
    Stringv(Vec<String>),
}

type GioCopyAttributes = BTreeMap<String, CopyAttributeValue>;

struct CopyMetadataSnapshot {
    target_metadata: std::fs::Metadata,
    attributes_query: String,
    gio_attributes: GioCopyAttributes,
    extended_attributes: ExtendedAttributes,
}

fn copy_attribute_value(info: &gio::FileInfo, name: &str) -> io::Result<CopyAttributeValue> {
    use gio::FileAttributeType;

    let missing = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("GIO omitted bookmark metadata attribute {name}"),
        )
    };
    match info.attribute_type(name) {
        FileAttributeType::String => info
            .attribute_string(name)
            .map(|value| CopyAttributeValue::String(value.to_string()))
            .ok_or_else(missing),
        FileAttributeType::ByteString => info
            .attribute_byte_string(name)
            .map(|value| CopyAttributeValue::ByteString(value.as_bytes().to_vec()))
            .ok_or_else(missing),
        FileAttributeType::Boolean => Ok(CopyAttributeValue::Boolean(info.boolean(name))),
        FileAttributeType::Uint32 => Ok(CopyAttributeValue::Uint32(info.attribute_uint32(name))),
        FileAttributeType::Int32 => Ok(CopyAttributeValue::Int32(info.attribute_int32(name))),
        FileAttributeType::Uint64 => Ok(CopyAttributeValue::Uint64(info.attribute_uint64(name))),
        FileAttributeType::Int64 => Ok(CopyAttributeValue::Int64(info.attribute_int64(name))),
        FileAttributeType::Stringv => Ok(CopyAttributeValue::Stringv(
            info.attribute_stringv(name)
                .iter()
                .map(|value| value.to_string())
                .collect(),
        )),
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("cannot verify GIO metadata attribute {name}"),
        )),
    }
}

fn is_copy_metadata_attribute(name: &str) -> bool {
    name.starts_with("unix::")
        || name.starts_with("time::")
        || name.starts_with("xattr::")
        || name.starts_with("xattr-sys::")
}

fn is_access_time_attribute(name: &str) -> bool {
    name.starts_with("time::access")
}

fn same_metadata_except_access_time(
    current: &GioCopyAttributes,
    expected: &GioCopyAttributes,
) -> bool {
    current
        .iter()
        .filter(|(name, _)| !is_access_time_attribute(name))
        .eq(expected
            .iter()
            .filter(|(name, _)| !is_access_time_attribute(name)))
}

fn query_gio_copy_attributes(
    path: &std::path::Path,
    attributes_query: &str,
) -> io::Result<GioCopyAttributes> {
    let info = gio::File::for_path(path)
        .query_info(
            attributes_query,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            None::<&gio::Cancellable>,
        )
        .map_err(|error| io::Error::other(format!("cannot query bookmark metadata: {error}")))?;
    let mut attributes = GioCopyAttributes::new();
    for name in info.list_attributes(None) {
        let name = name.as_str();
        if is_copy_metadata_attribute(name) {
            attributes.insert(name.to_owned(), copy_attribute_value(&info, name)?);
        }
    }

    #[cfg(unix)]
    for required in ["unix::mode", "unix::uid", "unix::gid"] {
        if !attributes.contains_key(required) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("GIO did not report essential bookmark metadata {required}"),
            ));
        }
    }
    Ok(attributes)
}

fn apply_gio_copy_attributes(
    source: &std::path::Path,
    destination: &gio::File,
    attributes_query: &str,
) -> io::Result<()> {
    let source_file = gio::File::for_path(source);
    let info = source_file
        .query_info(
            attributes_query,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            None::<&gio::Cancellable>,
        )
        .map_err(|error| io::Error::other(format!("cannot query bookmark metadata: {error}")))?;

    // build_attribute_list_for_copy() always includes standard::size for normal
    // file-copy bookkeeping. It is not settable metadata and GIO's
    // copy_attributes() attempts to set it on local files, failing with
    // "standard::size not supported". Keep only the attributes that the
    // destination advertised as copyable; set_attributes_from_info() applies
    // those source values to the destination (source -> destination).
    for name in info.list_attributes(None) {
        if !is_copy_metadata_attribute(name.as_str()) {
            info.remove_attribute(name.as_str());
        }
    }
    destination
        .set_attributes_from_info(
            &info,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            None::<&gio::Cancellable>,
        )
        .map_err(|error| io::Error::other(format!("cannot copy bookmark metadata: {error}")))
}

fn ensure_metadata_matches(
    source_attributes: &GioCopyAttributes,
    staged_attributes: &GioCopyAttributes,
    source_xattrs: &ExtendedAttributes,
    staged_xattrs: &ExtendedAttributes,
) -> io::Result<()> {
    if source_attributes != staged_attributes || source_xattrs != staged_xattrs {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bookmark metadata was not copied faithfully; refusing to replace the original",
        ))
    } else {
        Ok(())
    }
}

fn copy_and_verify_metadata(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> io::Result<Option<CopyMetadataSnapshot>> {
    let source_before = match metadata_without_symlink(source)? {
        Some(metadata) => metadata,
        None => return Ok(None),
    };
    let destination_file = gio::File::for_path(destination);
    let flags = gio::FileCopyFlags::ALL_METADATA | gio::FileCopyFlags::NOFOLLOW_SYMLINKS;
    // Use GIO's ALL_METADATA set for the destination; the copy helper below
    // queries `source` and applies those values to `destination`.
    let attributes_query = destination_file
        .build_attribute_list_for_copy(flags, None::<&gio::Cancellable>)
        .map_err(|error| {
            io::Error::other(format!("cannot prepare bookmark metadata copy: {error}"))
        })?
        .to_string();
    let source_attributes = query_gio_copy_attributes(source, &attributes_query)?;
    let (xattr_version, extended_attributes) = snapshot_extended_attributes(source)?;
    if !same_metadata_version(&source_before, &xattr_version) {
        return Ok(None);
    }

    apply_gio_copy_attributes(source, &destination_file, &attributes_query)?;

    let staged_attributes = query_gio_copy_attributes(destination, &attributes_query)?;
    let (_, staged_extended_attributes) = snapshot_extended_attributes(destination)?;
    let source_after = metadata_without_symlink(source)?;
    let Some(source_after) = source_after else {
        return Ok(None);
    };
    if !same_metadata_version(&source_before, &source_after) {
        return Ok(None);
    }
    ensure_metadata_matches(
        &source_attributes,
        &staged_attributes,
        &extended_attributes,
        &staged_extended_attributes,
    )?;

    Ok(Some(CopyMetadataSnapshot {
        target_metadata: source_after,
        attributes_query: attributes_query.to_string(),
        gio_attributes: source_attributes,
        extended_attributes,
    }))
}

fn current_metadata_matches(
    path: &std::path::Path,
    expected: &CopyMetadataSnapshot,
) -> io::Result<bool> {
    let before = metadata_without_symlink(path)?;
    let Some(before) = before else {
        return Ok(false);
    };
    if !same_metadata_version(&expected.target_metadata, &before) {
        return Ok(false);
    }

    let gio_attributes = query_gio_copy_attributes(path, &expected.attributes_query)?;
    let (xattr_version, extended_attributes) = snapshot_extended_attributes(path)?;
    let after = metadata_without_symlink(path)?;
    let Some(after) = after else {
        return Ok(false);
    };
    if !same_metadata_version(&before, &xattr_version) || !same_metadata_version(&before, &after) {
        return Ok(false);
    }

    // Reading the destination to compare its contents updates atime on some
    // filesystems, so it is copied to the stage but not a useful conflict key.
    Ok(
        same_metadata_except_access_time(&gio_attributes, &expected.gio_attributes)
            && extended_attributes == expected.extended_attributes,
    )
}

/// Opens an existing destination exactly as an in-place write would, but without
/// truncate or write. The descriptor also lets us reject a path swap immediately
/// before the final compare-and-rename.
fn open_target_for_write_check(
    path: &std::path::Path,
    expected: &std::fs::Metadata,
) -> io::Result<Option<std::fs::File>> {
    let before = metadata_without_symlink(path)?;
    let Some(before) = before else {
        return Ok(None);
    };
    if !same_metadata_version(expected, &before) {
        return Ok(None);
    }

    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;

        // Linux O_NOFOLLOW: do not open a symlink swapped in after lstat.
        options.custom_flags(0o400000);
    }
    #[cfg(not(target_os = "linux"))]
    return Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "cannot safely check write access without O_NOFOLLOW",
    ));

    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let opened_metadata = file.metadata()?;
    let after = metadata_without_symlink(path)?;
    let Some(after) = after else {
        return Ok(None);
    };
    if !same_metadata_version(expected, &opened_metadata)
        || !same_metadata_version(expected, &after)
    {
        return Ok(None);
    }
    Ok(Some(file))
}

fn open_and_sync_directory(path: &std::path::Path) -> io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        let directory = std::fs::File::open(path)?;
        directory.sync_all()?;
        Ok(directory)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "directory fsync is unavailable on this platform",
        ))
    }
}

fn sync_after_replace(
    staging: &PrivateStagingDir,
    staging_directory: &std::fs::File,
    parent_directory: &std::fs::File,
) -> io::Result<()> {
    let cleanup_error = std::fs::remove_dir(&staging.path).err();
    let staging_sync_error = staging_directory.sync_all().err();
    let parent_sync_error = parent_directory.sync_all().err();
    let errors = [cleanup_error, staging_sync_error, parent_sync_error]
        .into_iter()
        .flatten()
        .map(|error| error.to_string())
        .collect::<Vec<_>>();

    if errors.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "bookmarks were replaced, but cleanup or directory fsync failed; the update is already committed and may not survive a crash: {}",
            errors.join("; ")
        )))
    }
}

fn atomic_write_if_unchanged<F>(
    path: &std::path::Path,
    text: &str,
    expected: Option<&[u8]>,
    before_compare: F,
) -> io::Result<bool>
where
    F: FnOnce(),
{
    use std::io::Write;

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(parent)?;
    let staging = create_private_staging_dir(parent)?;
    let temp = staging.file_path();
    let mut file = create_staging_file(&staging)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;

    let metadata = match metadata_without_symlink(path)? {
        Some(_) => match copy_and_verify_metadata(path, &temp)? {
            Some(metadata) => Some(metadata),
            None => return Ok(false),
        },
        None => None,
    };
    // Metadata may have widened the staged inode's permissions, but it remains
    // behind the 0700 staging directory until the same-filesystem rename.
    file.sync_all()?;

    // Refuse to commit on filesystems which cannot sync directories. Sync both
    // directory entries before and after the rename for crash durability.
    let parent_directory = open_and_sync_directory(parent)?;
    let staging_directory = open_and_sync_directory(&staging.path)?;

    before_compare();
    let _write_permission_check = match &metadata {
        Some(metadata) => match open_target_for_write_check(path, &metadata.target_metadata)? {
            Some(file) => Some(file),
            None => return Ok(false),
        },
        None => None,
    };

    match read_snapshot_for_commit(path)? {
        CommitSnapshot::Stable(contents) if contents.as_deref() == expected => {}
        CommitSnapshot::Stable(_) | CommitSnapshot::Changed => return Ok(false),
    }
    match &metadata {
        Some(metadata) if !current_metadata_matches(path, metadata)? => return Ok(false),
        None if metadata_without_symlink(path)?.is_some() => return Ok(false),
        _ => {}
    }

    // This last stat checks that the open permission-check descriptor still
    // names the destination we compared; a writer can still race after it.
    if let (Some(metadata), Some(open_file)) = (&metadata, &_write_permission_check) {
        if !same_metadata_version(&metadata.target_metadata, &open_file.metadata()?) {
            return Ok(false);
        }
    }

    std::fs::rename(&temp, path)?;
    sync_after_replace(&staging, &staging_directory, &parent_directory)?;
    Ok(true)
}

const MAX_UPDATE_ATTEMPTS: usize = 8;

/// Applies an idempotent text mutation against a fresh snapshot after each
/// detected conflict. `before_compare` is a narrow test seam used to model an
/// external writer at the point where the staged file is ready to commit.
fn update_with_retry<T, M, H>(
    path: &std::path::Path,
    mut mutation: M,
    mut before_compare: H,
) -> io::Result<Option<T>>
where
    M: FnMut(&str) -> Option<(String, T)>,
    H: FnMut(usize, &std::path::Path),
{
    for attempt in 0..MAX_UPDATE_ATTEMPTS {
        let snapshot = read_snapshot(path)?;
        let text = match &snapshot {
            Some(bytes) => String::from_utf8(bytes.clone())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
            None => String::new(),
        };
        let Some((updated, result)) = mutation(&text) else {
            return Ok(None);
        };

        let committed = atomic_write_if_unchanged(path, &updated, snapshot.as_deref(), || {
            before_compare(attempt, path)
        })?;
        if committed {
            return Ok(Some(result));
        }
    }

    Err(io::Error::other(
        "bookmarks changed repeatedly while saving; refusing to overwrite",
    ))
}

pub fn read() -> Vec<(String, String)> {
    read_from(&path())
}

pub fn is_pinned(uri: &str) -> bool {
    let needle = normalize(uri);
    read().iter().any(|(_, u)| normalize(u) == needle)
}

/// Adds the pin (creates file and folders if missing). Idempotent.
pub fn pin_to(path: &std::path::Path, name: &str, uri: &str) -> Result<(), glib::Error> {
    pin_to_with_hook(path, name, uri, |_, _| {})
}

fn pin_to_with_hook<H>(
    path: &std::path::Path,
    name: &str,
    uri: &str,
    before_compare: H,
) -> Result<(), glib::Error>
where
    H: FnMut(usize, &std::path::Path),
{
    if uri == crate::FAVORITES_URI || uri == crate::ADD_BOOKMARK_URI {
        return Err(glib::Error::new(
            gio::IOErrorEnum::InvalidArgument,
            "Cannot bookmark an internal Kito destination",
        ));
    }
    let needle = normalize(uri);
    let stored_uri = if uri.ends_with(":///") {
        uri
    } else {
        uri.trim_end_matches('/')
    };
    let label = name.split_whitespace().collect::<Vec<_>>().join(" ");
    update_with_retry(
        path,
        |current| {
            if current
                .lines()
                .filter_map(uri_in_line)
                .any(|item| normalize(item) == needle)
            {
                return None;
            }
            let mut updated = current.to_string();
            if !updated.is_empty() && !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str(&format!("{stored_uri} {label}\n"));
            Some((updated, ()))
        },
        before_compare,
    )
    .map(|_| ())
    .map_err(|error| glib::Error::new(gio::IOErrorEnum::Failed, &format!("Cannot pin: {error}")))
}

pub fn pin(name: &str, uri: &str) -> Result<(), glib::Error> {
    pin_to(&path(), name, uri)
}

/// Removes the pin. Returns `true` if it was there.
pub fn unpin_from(path: &std::path::Path, uri: &str) -> Result<bool, glib::Error> {
    let needle = normalize(uri);
    update_with_retry(
        path,
        |text| {
            let mut existed = false;
            let mut output = String::with_capacity(text.len());
            for line in text.split_inclusive('\n') {
                let content = line.strip_suffix('\n').unwrap_or(line);
                let remove = uri_in_line(content).is_some_and(|item| normalize(item) == needle);
                existed |= remove;
                if !remove {
                    output.push_str(line);
                }
            }
            existed.then_some((output, true))
        },
        |_, _| {},
    )
    .map(|result| result.unwrap_or(false))
    .map_err(|error| glib::Error::new(gio::IOErrorEnum::Failed, &format!("Cannot unpin: {error}")))
}

pub fn unpin(uri: &str) -> Result<bool, glib::Error> {
    unpin_from(&path(), uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_unpin_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bookmarks");
        assert!(!is_pinned_in(&file, "file:///tmp/docs"));

        pin_to(&file, "Docs", "file:///tmp/docs/").unwrap();
        pin_to(&file, "Docs", "file:///tmp/docs").unwrap(); // idempotent
        assert_eq!(read_from(&file).len(), 1);

        assert!(unpin_from(&file, "file:///tmp/docs").unwrap());
        assert!(!unpin_from(&file, "file:///tmp/docs").unwrap());
        assert!(read_from(&file).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn staging_directory_and_file_start_private() {
        use std::os::unix::fs::MetadataExt;

        let parent = tempfile::tempdir().unwrap();
        let staging = create_private_staging_dir(parent.path()).unwrap();
        let file = create_staging_file(&staging).unwrap();

        assert_eq!(
            std::fs::metadata(&staging.path).unwrap().mode() & 0o777,
            0o700
        );
        assert_eq!(file.metadata().unwrap().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(staging.file_path()).unwrap().len(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn newly_created_bookmarks_remain_user_only() {
        use std::os::unix::fs::MetadataExt;

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bookmarks");
        pin_to(&file, "New", "file:///tmp/new").unwrap();

        assert_eq!(std::fs::metadata(&file).unwrap().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn replacements_preserve_mode_owner_group_acl_and_xattrs() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bookmarks");
        std::fs::write(&file, "# existing\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
        let gio_file = gio::File::for_path(&file);

        let user_xattr_supported = set_test_xattr(
            &gio_file,
            "xattr::kito-test-value",
            r"\x00\x7f\xff\x5c",
            "user xattr",
        );
        let acl_supported = set_test_xattr(
            &gio_file,
            "xattr-sys::system.posix_acl_access",
            &test_acl_value(),
            "POSIX ACL",
        );

        let before_metadata = std::fs::metadata(&file).unwrap();
        let before_xattrs = snapshot_extended_attributes(&file).unwrap().1;
        if user_xattr_supported {
            assert!(before_xattrs.contains_key(b"user.kito-test-value".as_slice()));
        }
        if acl_supported {
            assert!(before_xattrs.contains_key(b"system.posix_acl_access".as_slice()));
        }

        pin_to(&file, "New pin", "file:///tmp/new-pin").unwrap();

        let after_metadata = std::fs::metadata(&file).unwrap();
        let after_xattrs = snapshot_extended_attributes(&file).unwrap().1;
        assert_eq!(after_metadata.mode(), before_metadata.mode());
        assert_eq!(after_metadata.uid(), before_metadata.uid());
        assert_eq!(after_metadata.gid(), before_metadata.gid());
        assert_eq!(
            after_metadata.modified().unwrap(),
            before_metadata.modified().unwrap()
        );
        assert_eq!(after_xattrs, before_xattrs);
    }

    #[cfg(unix)]
    #[test]
    fn read_only_destination_fails_without_replacing_or_leaking_staging() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bookmarks");
        let original = "file:///tmp/existing Existing\n";
        std::fs::write(&file, original).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();

        // Root can open mode-0444 files for writing, just as an in-place write
        // would succeed with those credentials, so this assertion is inapplicable.
        if std::fs::OpenOptions::new().write(true).open(&file).is_ok() {
            eprintln!(
                "skipping readonly-mode denial: current credentials can write mode-0444 files"
            );
            return;
        }

        assert!(pin_to(&file, "New", "file:///tmp/new").is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
        assert_eq!(std::fs::metadata(&file).unwrap().mode() & 0o777, 0o444);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mismatched_staged_metadata_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let staged = dir.path().join("staged");
        std::fs::write(&source, "original\n").unwrap();
        std::fs::write(&staged, "replacement\n").unwrap();
        let gio = gio::File::for_path(&source);
        let xattr_added = set_test_xattr(
            &gio,
            "xattr::kito-test-mismatch",
            "value",
            "metadata mismatch xattr",
        );
        if !xattr_added {
            eprintln!("skipping metadata mismatch test: user xattrs are unavailable");
            return;
        }

        let source_xattrs = snapshot_extended_attributes(&source).unwrap().1;
        let staged_xattrs = snapshot_extended_attributes(&staged).unwrap().1;
        assert_ne!(source_xattrs, staged_xattrs);
        assert!(source_xattrs.contains_key(b"user.kito-test-mismatch".as_slice()));
        let flags = gio::FileCopyFlags::ALL_METADATA | gio::FileCopyFlags::NOFOLLOW_SYMLINKS;
        let attributes_query = gio::File::for_path(&staged)
            .build_attribute_list_for_copy(flags, None::<&gio::Cancellable>)
            .unwrap();
        let source_gio_attributes = query_gio_copy_attributes(&source, &attributes_query).unwrap();
        let staged_gio_attributes = query_gio_copy_attributes(&staged, &attributes_query).unwrap();
        assert!(ensure_metadata_matches(
            &source_gio_attributes,
            &source_gio_attributes,
            &source_xattrs,
            &staged_xattrs,
        )
        .is_err());
        assert!(ensure_metadata_matches(
            &source_gio_attributes,
            &staged_gio_attributes,
            &source_xattrs,
            &staged_xattrs,
        )
        .is_err());
    }

    #[test]
    fn safe_updates_preserve_unknown_lines_comments_and_labels() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bookmarks");
        let existing = "# managed elsewhere\nsmb://host/share  Label   with spaces\nunknown unparsed payload\n";
        std::fs::write(&file, existing).unwrap();
        pin_to(&file, "Folder bookmark", "file:///tmp/folder").unwrap();
        pin_to(&file, "Duplicate", "file:///tmp/folder/").unwrap();
        pin_to(&file, "File bookmark", "file:///tmp/file.txt").unwrap();
        let after_pin = std::fs::read_to_string(&file).unwrap();
        assert!(after_pin.starts_with(existing));
        assert_eq!(
            after_pin
                .matches("file:///tmp/folder Folder bookmark")
                .count(),
            1
        );
        assert!(unpin_from(&file, "file:///tmp/folder").unwrap());
        assert!(unpin_from(&file, "file:///tmp/file.txt").unwrap());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), existing);
    }

    #[test]
    fn pin_reloads_and_preserves_a_concurrent_bookmark_writer() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bookmarks");
        let before = "# managed elsewhere\nunknown payload\n";
        let external_update = format!("{before}smb://host/share Nautilus bookmark\n");
        std::fs::write(&file, before).unwrap();

        pin_to_with_hook(
            &file,
            "Kito pin",
            "file:///tmp/kito-pin",
            |attempt, path| {
                if attempt == 0 {
                    // Deterministically model another process saving after our
                    // snapshot and after the new contents have been staged.
                    std::fs::write(path, &external_update).unwrap();
                }
            },
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            format!("{external_update}file:///tmp/kito-pin Kito pin\n")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pin_retries_when_metadata_changes_after_staging() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bookmarks");
        std::fs::write(&file, "# original\n").unwrap();
        let gio_file = gio::File::for_path(&file);
        let xattr = "xattr::kito-test-concurrent";
        if !set_test_xattr(&gio_file, xattr, "before", "concurrent xattr") {
            eprintln!("skipping metadata-conflict test: user xattrs are unavailable");
            return;
        }

        pin_to_with_hook(&file, "Kito pin", "file:///tmp/kito-pin", |attempt, _| {
            if attempt == 0 {
                gio_file
                    .set_attribute_string(
                        xattr,
                        "after",
                        gio::FileQueryInfoFlags::NONE,
                        None::<&gio::Cancellable>,
                    )
                    .unwrap();
            }
        })
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "# original\nfile:///tmp/kito-pin Kito pin\n"
        );
        assert_eq!(
            snapshot_extended_attributes(&file).unwrap().1[b"user.kito-test-concurrent".as_slice()],
            b"after"
        );
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn pin_detects_a_concurrent_create_when_bookmarks_were_missing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bookmarks");
        let external_update = "smb://host/share Nautilus bookmark\n";

        pin_to_with_hook(
            &file,
            "Kito pin",
            "file:///tmp/kito-pin",
            |attempt, path| {
                if attempt == 0 {
                    std::fs::write(path, external_update).unwrap();
                }
            },
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            format!("{external_update}file:///tmp/kito-pin Kito pin\n")
        );
    }

    #[test]
    fn repeated_conflicts_refuse_to_overwrite_the_latest_external_contents() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bookmarks");
        std::fs::write(&file, "initial\n").unwrap();

        let result = pin_to_with_hook(
            &file,
            "Kito pin",
            "file:///tmp/kito-pin",
            |attempt, path| {
                std::fs::write(path, format!("external-{attempt}\n")).unwrap();
            },
        );

        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            format!("external-{}\n", MAX_UPDATE_ATTEMPTS - 1)
        );
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn bookmark_mutations_reject_symlink_paths_without_touching_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target-bookmarks");
        let link = dir.path().join("bookmarks");
        let original = "file:///tmp/existing Existing\n";
        std::fs::write(&target, original).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(pin_to(&link, "New", "file:///tmp/new").is_err());
        assert!(unpin_from(&link, "file:///tmp/existing").is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), original);
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn unreadable_or_invalid_utf8_bookmarks_are_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let invalid = dir.path().join("invalid-bookmarks");
        let original = [0xff, 0xfe, b'\n'];
        std::fs::write(&invalid, original).unwrap();
        assert!(pin_to(&invalid, "New", "file:///tmp/new").is_err());
        assert!(unpin_from(&invalid, "file:///tmp/old").is_err());
        assert_eq!(std::fs::read(&invalid).unwrap(), original);

        // A directory path deterministically injects a read error, including
        // when tests run as root (unlike chmod-based permission fixtures).
        let directory = dir.path().join("directory-bookmarks");
        std::fs::create_dir(&directory).unwrap();
        assert!(pin_to(&directory, "New", "file:///tmp/new").is_err());
        assert!(unpin_from(&directory, "file:///tmp/new").is_err());
        assert!(directory.is_dir());
    }

    #[test]
    fn unlabeled_bookmarks_use_a_readable_basename() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bookmarks");
        let uri = gio::File::for_path(dir.path().join("Project Files")).uri();
        std::fs::write(&file, format!("{uri}\n")).unwrap();

        assert_eq!(
            read_from(&file),
            [("Project Files".to_string(), uri.to_string())]
        );
    }

    #[test]
    fn explicit_bookmark_labels_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bookmarks");
        std::fs::write(&file, "file:///tmp/project Project Files\n").unwrap();

        assert_eq!(
            read_from(&file),
            [(
                "Project Files".to_string(),
                "file:///tmp/project".to_string()
            )]
        );
    }

    fn is_pinned_in(path: &std::path::Path, uri: &str) -> bool {
        let needle = normalize(uri);
        read_from(path).iter().any(|(_, u)| normalize(u) == needle)
    }

    #[cfg(target_os = "linux")]
    fn set_test_xattr(file: &gio::File, attribute: &str, value: &str, label: &str) -> bool {
        match file.set_attribute_string(
            attribute,
            value,
            gio::FileQueryInfoFlags::NONE,
            None::<&gio::Cancellable>,
        ) {
            Ok(()) => true,
            Err(error)
                if error.matches(gio::IOErrorEnum::NotSupported)
                    || error.matches(gio::IOErrorEnum::PermissionDenied) =>
            {
                eprintln!("skipping {label} preservation assertion: {error}");
                false
            }
            Err(error) => panic!("could not set test {label}: {error}"),
        }
    }

    #[cfg(target_os = "linux")]
    fn test_acl_value() -> String {
        // Linux POSIX ACL xattr v2: owner rwx, named user r, group r, mask r,
        // other none. GIO's xattr API takes backslash-hex escaped bytes.
        let entries = [
            (0x01u16, 0x07u16, u32::MAX),
            (0x02u16, 0x04u16, 40_001u32),
            (0x04u16, 0x04u16, u32::MAX),
            (0x10u16, 0x04u16, u32::MAX),
            (0x20u16, 0x00u16, u32::MAX),
        ];
        let mut bytes = 2u32.to_le_bytes().to_vec();
        for (tag, permissions, id) in entries {
            bytes.extend(tag.to_le_bytes());
            bytes.extend(permissions.to_le_bytes());
            bytes.extend(id.to_le_bytes());
        }
        bytes.iter().map(|byte| format!("\\x{byte:02x}")).collect()
    }
}
