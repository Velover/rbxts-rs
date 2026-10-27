//! A file's stamp, asked of the file itself: what tells a later edit of a program file apart without reading it
//! (the source cache's reuse of a parsed file, a watch rebuild's check of what it parsed).
//!
//! **The file, not its directory.** A directory listing's times and sizes are copies NTFS keeps in the directory and
//! brings up to date when the writer closes the file: while an editor or a tool holds a file it has written open, a
//! listing still shows its old time and size. The stamp asks the file (`NtQueryFullAttributesFile` on Windows, one
//! system call that opens nothing the caller sees, about a fifth of what `std::fs::metadata`'s handle costs; `stat`
//! elsewhere).
//!
//! **The change time.** The modification time can be set back (`touch -d`, an archive or copy tool restoring times,
//! a version control checkout), and a same-size rewrite with its old time restored would read as unchanged. The change
//! time (NTFS's `ChangeTime`, POSIX `ctime`) is the file system's own: it moves with every write and every change of
//! the file's metadata, setting the modification time included, and only a deliberate call (a backup restore on
//! Windows) sets it back.
//!
//! **Settled.** Two edits within one tick of the file system's clock get the same times: a stamp taken when the change
//! time was that close ([`crate::read_check::settled_time`]) is not settled, and a file whose stamp is the same but
//! not settled is compared by its text.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::read_check::settled_time;

/// A file's modification and change times (nanoseconds since the Unix epoch, 0 when unknown) and size, and whether
/// the stamp was taken long enough after the change time to tell a later edit apart by the times alone.
#[derive(Clone, Copy, Debug)]
pub struct FileStamp {
    /// The modification time.
    pub modified: u128,
    /// The change time (NTFS's `ChangeTime`, POSIX `ctime`).
    pub changed: u128,
    /// The size in bytes.
    pub size: u64,
    /// The stamp was taken [`crate::read_check::settled_time`] after the change time: a later edit has other times.
    pub settled: bool,
}

impl FileStamp {
    /// The stamp of a file that never changes (a lib bundled in the binary).
    pub const FIXED: FileStamp = FileStamp { modified: 0, changed: 0, size: 0, settled: true };

    /// The same times and size (whether either stamp is settled aside).
    pub fn same(&self, other: &FileStamp) -> bool {
        self.modified == other.modified && self.changed == other.changed && self.size == other.size
    }

    fn new(modified: u128, changed: u128, size: u64) -> FileStamp {
        FileStamp { modified, changed, size, settled: settled_time(changed) != 0 }
    }
}

/// What a path is, asked of the path itself (links followed).
#[derive(Clone, Copy, Debug)]
pub enum PathStat {
    /// Nothing there (or not reachable).
    Missing,
    /// A directory.
    Dir,
    /// A file, with its stamp.
    File(FileStamp),
}

impl PathStat {
    /// The stamp of the file at `path`, if a file is there.
    pub fn file(self) -> Option<FileStamp> {
        match self {
            PathStat::File(stamp) => Some(stamp),
            _ => None,
        }
    }
}

/// Asks `path` (absolute) what it is now.
pub fn stat(path: &str) -> PathStat {
    #[cfg(windows)]
    if let Some(stat) = windows::stat(path) {
        return stat;
    }
    std_stat(path)
}

fn nanos(time: std::io::Result<SystemTime>) -> u128 {
    time.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos())
}

/// Through `std::fs::metadata` (a handle on Windows, `stat` elsewhere).
fn std_stat(path: &str) -> PathStat {
    let Ok(metadata) = std::fs::metadata(path) else { return PathStat::Missing };
    if metadata.is_dir() {
        return PathStat::Dir;
    }
    #[cfg(unix)]
    let changed = {
        use std::os::unix::fs::MetadataExt;
        u128::try_from(metadata.ctime()).map_or(0, |s| s * 1_000_000_000 + metadata.ctime_nsec() as u128)
    };
    #[cfg(not(unix))]
    let changed = 0;
    PathStat::File(FileStamp::new(nanos(metadata.modified()), changed, metadata.len()))
}

#[cfg(windows)]
mod windows {
    use super::{FileStamp, PathStat};

    #[repr(C)]
    #[derive(Default)]
    struct FileNetworkOpenInformation {
        creation_time: i64,
        last_access_time: i64,
        last_write_time: i64,
        change_time: i64,
        allocation_size: i64,
        end_of_file: i64,
        file_attributes: u32,
    }

    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum_length: u16,
        buffer: *const u16,
    }

    #[repr(C)]
    struct ObjectAttributes {
        length: u32,
        root_directory: isize,
        object_name: *const UnicodeString,
        attributes: u32,
        security_descriptor: *const u8,
        security_quality_of_service: *const u8,
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtQueryFullAttributesFile(attributes: *const ObjectAttributes, information: *mut FileNetworkOpenInformation)
        -> i32;
    }

    const OBJ_CASE_INSENSITIVE: u32 = 0x40;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const STATUS_OBJECT_NAME_NOT_FOUND: i32 = 0xC000_0034_u32 as i32;
    const STATUS_OBJECT_PATH_NOT_FOUND: i32 = 0xC000_003A_u32 as i32;
    const STATUS_NOT_A_DIRECTORY: i32 = 0xC000_0103_u32 as i32;
    /// 1601-01-01 to 1970-01-01 in 100 ns ticks.
    const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

    fn nanos(ticks: i64) -> u128 {
        u128::try_from(ticks - UNIX_EPOCH_TICKS).map_or(0, |t| t * 100)
    }

    /// `None` when the question cannot be put this way (not an absolute drive or UNC path, too long, or an answer other
    /// than the file's information or its absence): `std::fs::metadata` answers then.
    pub(super) fn stat(path: &str) -> Option<PathStat> {
        let path = path.replace('/', "\\");
        let nt_path = if let Some(rest) = path.strip_prefix("\\\\?\\") {
            format!("\\??\\{rest}")
        } else if let Some(rest) = path.strip_prefix("\\\\") {
            format!("\\??\\UNC\\{rest}")
        } else if path.as_bytes().get(1) == Some(&b':') && path.as_bytes().get(2) == Some(&b'\\') {
            format!("\\??\\{path}")
        } else {
            return None;
        };
        let wide: Vec<u16> = nt_path.encode_utf16().collect();
        let length = u16::try_from(wide.len() * 2).ok()?;
        let name = UnicodeString { length, maximum_length: length, buffer: wide.as_ptr() };
        let attributes = ObjectAttributes {
            length: size_of::<ObjectAttributes>() as u32,
            root_directory: 0,
            object_name: &name,
            attributes: OBJ_CASE_INSENSITIVE,
            security_descriptor: std::ptr::null(),
            security_quality_of_service: std::ptr::null(),
        };
        let mut information = FileNetworkOpenInformation::default();
        // SAFETY: the attributes point at a counted UTF-16 name that outlives the call, and the information is the
        // FILE_NETWORK_OPEN_INFORMATION the call fills
        let status = unsafe { NtQueryFullAttributesFile(&attributes, &mut information) };
        if status >= 0 {
            if information.file_attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                return Some(PathStat::Dir);
            }
            let size = u64::try_from(information.end_of_file).unwrap_or(0);
            return Some(PathStat::File(FileStamp::new(
                nanos(information.last_write_time),
                nanos(information.change_time),
                size,
            )));
        }
        matches!(status, STATUS_OBJECT_NAME_NOT_FOUND | STATUS_OBJECT_PATH_NOT_FOUND | STATUS_NOT_A_DIRECTORY)
            .then_some(PathStat::Missing)
    }
}

#[cfg(test)]
mod tests {
    use super::{PathStat, stat};
    use std::time::{Duration, SystemTime};

    /// A same-size rewrite with the old modification time set back has another stamp; a file held open after a
    /// write has its new size (a listing shows the old one until the writer closes it); a directory and a missing
    /// path are told apart.
    #[test]
    fn stamps_tell_edits_apart() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("rbxts-rs-stamp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.ts");
        let path = file.to_string_lossy().into_owned();
        std::fs::write(&file, "export const a = 1;").unwrap();
        let old = SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(old).unwrap();
        let before = stat(&path).file().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&file, "export const a = 2;").unwrap();
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(old).unwrap();
        let after = stat(&path).file().unwrap();
        assert_eq!((before.modified, before.size), (after.modified, after.size));
        assert!(!before.same(&after), "the change time tells them apart");
        assert!(!after.settled, "changed just now");

        let mut writer = std::fs::File::options().append(true).open(&file).unwrap();
        writer.write_all(b"\n").unwrap();
        writer.flush().unwrap();
        assert_eq!(stat(&path).file().unwrap().size, 20);
        drop(writer);

        assert!(matches!(stat(&dir.to_string_lossy()), PathStat::Dir));
        assert!(matches!(stat(&dir.join("missing.ts").to_string_lossy()), PathStat::Missing));
        assert!(matches!(stat(&dir.join("missing/deeper.ts").to_string_lossy()), PathStat::Missing));
        assert!(matches!(stat(&file.join("below").to_string_lossy()), PathStat::Missing));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
