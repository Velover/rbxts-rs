//! The few `path` functions of Node the TS transformer's results depend on, with Node's semantics (`path.win32` on
//! Windows, `path.posix` elsewhere): `resolve`, `relative` (case-insensitive on Windows), `dirname`, `join`, and
//! `pathToFileURL`. Strings in, strings out, as the TS code has them; the compiler's own path type can wrap these.

#[cfg(windows)]
pub use win32::*;

#[cfg(not(windows))]
pub use posix::*;

/// `isPathDescendantOf(filePath, dirPath)` from the TS transformer: the same path, or a relative path from the
/// directory that neither starts with `..` nor is absolute. (A child named `..x` counts as outside, as in the TS.)
pub fn is_path_descendant_of(file_path: &str, dir_path: &str) -> bool {
    let relative = relative(dir_path, file_path);
    dir_path == file_path || (!relative.starts_with("..") && !is_absolute(&relative))
}

/// `path.relative(from, to).replace(/\\/g, "/")`.
pub fn relative_slash(from: &str, to: &str) -> String {
    relative(from, to).replace('\\', "/")
}

/// `url.pathToFileURL(path).href` for an absolute path.
pub fn path_to_file_url(path: &str) -> String {
    let resolved = resolve(&[path]).replace('\\', "/");
    let mut out = String::from("file://");
    if !resolved.starts_with('/') {
        out.push('/');
    }
    for c in resolved.chars() {
        let keep = c.is_ascii_alphanumeric() || "!$&'()*+,-./:;=@[]^_|~".contains(c);
        if keep {
            out.push(c);
        } else {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

/// Node's `path.win32`.
pub mod win32 {
    fn is_sep(c: char) -> bool {
        c == '\\' || c == '/'
    }

    /// The root of a path (`C:\`, `C:`, `\`, `\\server\share\`) and the rest.
    fn split_root(p: &str) -> (String, &str) {
        let b = p.as_bytes();
        if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
            if b.len() >= 3 && is_sep(b[2] as char) {
                return (format!("{}\\", &p[..2]), &p[3..]);
            }
            return (p[..2].to_owned(), &p[2..]);
        }
        if b.len() >= 2 && is_sep(b[0] as char) && is_sep(b[1] as char) {
            // A UNC path: \\server\share\ is the root.
            let rest = &p[2..];
            let mut parts = rest.splitn(3, ['\\', '/']);
            let server = parts.next().unwrap_or("");
            let share = parts.next().unwrap_or("");
            let tail = parts.next().unwrap_or("");
            return (format!("\\\\{server}\\{share}\\"), tail);
        }
        if !b.is_empty() && is_sep(b[0] as char) {
            return ("\\".to_owned(), &p[1..]);
        }
        (String::new(), p)
    }

    pub fn is_absolute(p: &str) -> bool {
        let (root, _) = split_root(p);
        root.ends_with('\\')
    }

    /// Segments without `.` and with `..` applied (kept at the front of a relative path).
    fn normalize_segments(rest: &str, absolute: bool) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for seg in rest.split(['\\', '/']) {
            match seg {
                "" | "." => {}
                ".." => {
                    if out.last().is_some_and(|s| *s != "..") {
                        out.pop();
                    } else if !absolute {
                        out.push("..");
                    }
                }
                s => out.push(s),
            }
        }
        out
    }

    /// `path.win32.resolve(...paths)`, against the process's current directory when no path is absolute.
    pub fn resolve(paths: &[&str]) -> String {
        let mut device = String::new();
        let mut tail = String::new();
        let mut absolute = false;
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let all: Vec<&str> = std::iter::once(cwd.as_str())
            .chain(paths.iter().copied())
            .collect();
        for p in all.iter().rev() {
            if p.is_empty() {
                continue;
            }
            let (root, rest) = split_root(p);
            let root_device = root.trim_end_matches('\\').to_owned();
            let is_abs = root.ends_with('\\');
            if !device.is_empty()
                && !root_device.is_empty()
                && !root_device.eq_ignore_ascii_case(&device)
            {
                continue;
            }
            if device.is_empty() && !root_device.is_empty() {
                device = root_device;
            }
            if !absolute {
                tail = if tail.is_empty() {
                    rest.to_owned()
                } else {
                    format!("{rest}\\{tail}")
                };
                absolute = is_abs;
            }
            if absolute && !device.is_empty() {
                break;
            }
        }
        let segments = normalize_segments(&tail, absolute);
        let joined = segments.join("\\");
        if absolute {
            format!("{device}\\{joined}")
        } else {
            format!("{device}{joined}")
        }
    }

    /// `path.win32.relative(from, to)`: compared without case; `to` itself when the roots differ.
    pub fn relative(from: &str, to: &str) -> String {
        let from = resolve(&[from]);
        let to = resolve(&[to]);
        if from == to || from.to_lowercase() == to.to_lowercase() {
            return String::new();
        }
        let (from_root, from_rest) = split_root(&from);
        let (to_root, to_rest) = split_root(&to);
        if from_root.to_lowercase() != to_root.to_lowercase() {
            return to;
        }
        let a: Vec<&str> = from_rest.split('\\').filter(|s| !s.is_empty()).collect();
        let b: Vec<&str> = to_rest.split('\\').filter(|s| !s.is_empty()).collect();
        let common = a
            .iter()
            .zip(&b)
            .take_while(|(x, y)| x.to_lowercase() == y.to_lowercase())
            .count();
        let mut parts: Vec<&str> = vec![".."; a.len() - common];
        parts.extend(&b[common..]);
        parts.join("\\")
    }

    /// `path.win32.dirname(p)`.
    pub fn dirname(p: &str) -> String {
        let (root, rest) = split_root(p);
        let rest = rest.trim_end_matches(['\\', '/']);
        match rest.rfind(['\\', '/']) {
            Some(i) => format!("{root}{}", rest[..i].trim_end_matches(['\\', '/'])),
            None if root.is_empty() => ".".to_owned(),
            None => root,
        }
    }

    /// `path.win32.join(...paths)`.
    pub fn join(paths: &[&str]) -> String {
        let joined = paths
            .iter()
            .filter(|p| !p.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("\\");
        if joined.is_empty() {
            return ".".to_owned();
        }
        normalize(&joined)
    }

    /// `path.win32.normalize(p)`.
    pub fn normalize(p: &str) -> String {
        let (root, rest) = split_root(p);
        let absolute = root.ends_with('\\');
        let trailing = rest.ends_with(['\\', '/']);
        let mut joined = normalize_segments(rest, absolute).join("\\");
        if joined.is_empty() && !absolute {
            joined = ".".to_owned();
        }
        if trailing && !joined.is_empty() {
            joined.push('\\');
        }
        format!("{root}{joined}")
    }

    /// `path.win32.parse(p).name`: the base name without its last extension.
    pub fn parse_name(p: &str) -> String {
        let base = p.trim_end_matches(['\\', '/']);
        let base = &base[base.rfind(['\\', '/', ':']).map_or(0, |i| i + 1)..];
        match base.rfind('.') {
            Some(i) if i > 0 => base[..i].to_owned(),
            _ => base.to_owned(),
        }
    }
}

/// Node's `path.posix`.
pub mod posix {
    pub fn is_absolute(p: &str) -> bool {
        p.starts_with('/')
    }

    fn normalize_segments(rest: &str, absolute: bool) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for seg in rest.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    if out.last().is_some_and(|s| *s != "..") {
                        out.pop();
                    } else if !absolute {
                        out.push("..");
                    }
                }
                s => out.push(s),
            }
        }
        out
    }

    pub fn resolve(paths: &[&str]) -> String {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut tail = String::new();
        for p in std::iter::once(cwd.as_str())
            .chain(paths.iter().copied())
            .collect::<Vec<_>>()
            .iter()
            .rev()
        {
            if p.is_empty() {
                continue;
            }
            tail = if tail.is_empty() {
                (*p).to_owned()
            } else {
                format!("{p}/{tail}")
            };
            if p.starts_with('/') {
                break;
            }
        }
        format!("/{}", normalize_segments(&tail, true).join("/"))
    }

    pub fn relative(from: &str, to: &str) -> String {
        let from = resolve(&[from]);
        let to = resolve(&[to]);
        if from == to {
            return String::new();
        }
        let a: Vec<&str> = from.split('/').filter(|s| !s.is_empty()).collect();
        let b: Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
        let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let mut parts: Vec<&str> = vec![".."; a.len() - common];
        parts.extend(&b[common..]);
        parts.join("/")
    }

    pub fn dirname(p: &str) -> String {
        let rest = p.trim_end_matches('/');
        match rest.rfind('/') {
            Some(0) => "/".to_owned(),
            Some(i) => rest[..i].trim_end_matches('/').to_owned(),
            None if p.starts_with('/') => "/".to_owned(),
            None => ".".to_owned(),
        }
    }

    pub fn join(paths: &[&str]) -> String {
        let joined = paths
            .iter()
            .filter(|p| !p.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("/");
        if joined.is_empty() {
            return ".".to_owned();
        }
        normalize(&joined)
    }

    pub fn normalize(p: &str) -> String {
        let absolute = p.starts_with('/');
        let trailing = p.ends_with('/');
        let mut joined = normalize_segments(p, absolute).join("/");
        if joined.is_empty() && !absolute {
            joined = ".".to_owned();
        }
        if trailing && !joined.is_empty() {
            joined.push('/');
        }
        if absolute {
            format!("/{joined}")
        } else {
            joined
        }
    }

    pub fn parse_name(p: &str) -> String {
        let base = p.trim_end_matches('/');
        let base = &base[base.rfind('/').map_or(0, |i| i + 1)..];
        match base.rfind('.') {
            Some(i) if i > 0 => base[..i].to_owned(),
            _ => base.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::win32;

    #[test]
    fn win32_relative() {
        assert_eq!(win32::relative("C:\\p", "C:\\p\\out\\a"), "out\\a");
        assert_eq!(
            win32::relative("C:\\p\\places\\x", "C:\\p\\out\\a"),
            "..\\..\\out\\a"
        );
        assert_eq!(win32::relative("C:\\p", "c:\\P"), "");
        assert_eq!(win32::relative("C:\\p", "D:\\out"), "D:\\out");
        assert_eq!(win32::relative("C:\\p\\a", "C:\\p\\a b\\x"), "..\\a b\\x");
    }

    #[test]
    fn win32_misc() {
        assert_eq!(win32::dirname("C:\\p\\a.ts"), "C:\\p");
        assert_eq!(win32::dirname("C:\\p"), "C:\\");
        assert_eq!(
            win32::join(&["C:\\p", "places\\a", "..", "f.json"]),
            "C:\\p\\places\\f.json"
        );
        assert_eq!(
            win32::resolve(&["C:\\p\\places\\a", "config/fw.json"]),
            "C:\\p\\places\\a\\config\\fw.json"
        );
        assert_eq!(
            win32::resolve(&["C:\\p\\places\\a", "..\\..\\other.json"]),
            "C:\\p\\other.json"
        );
        assert_eq!(win32::parse_name("C:\\p\\src\\types.d.ts"), "types.d");
        assert_eq!(win32::parse_name("C:\\p\\src\\.ts"), ".ts");
    }
}
