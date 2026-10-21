//! Where the transformer's own files come from: the config file, `.env` files, `package.json`s and `flamework.build`s.
//!
//! In a build they are read through the compiler (`SetupCx::read_input`), which records each one as a build input,
//! present or not, so that incremental and watch builds notice a change (DESIGN §14). Tests and tools read the disk
//! directly ([`FsInputs`]).

/// A source of files, read whole as UTF-8.
pub trait Inputs {
    /// The file's text, or `None` when it does not exist or cannot be read (a directory included). The text is as
    /// stored, byte order mark included.
    fn read(&mut self, path: &str) -> Option<String>;

    /// Whether a readable file is there. Reads it: every existence check is a recorded input.
    fn exists(&mut self, path: &str) -> bool {
        self.read(path).is_some()
    }

    /// A variable of the process environment.
    fn env(&mut self, name: &str) -> Option<String>;
}

/// The disk, unrecorded.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsInputs;

impl Inputs for FsInputs {
    fn read(&mut self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn exists(&mut self, path: &str) -> bool {
        std::path::Path::new(path).is_file()
    }

    fn env(&mut self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// The disk, with a given process environment.
pub struct FsWithEnv<'e>(pub &'e crate::env::Env);

impl Inputs for FsWithEnv<'_> {
    fn read(&mut self, path: &str) -> Option<String> {
        FsInputs.read(path)
    }

    fn exists(&mut self, path: &str) -> bool {
        FsInputs.exists(path)
    }

    fn env(&mut self, name: &str) -> Option<String> {
        self.0.get(name).map(str::to_owned)
    }
}

/// `ts.sys.readFile`: the text without a UTF-8 byte order mark.
pub fn strip_bom(text: String) -> String {
    match text.strip_prefix('\u{feff}') {
        Some(rest) => rest.to_owned(),
        None => text,
    }
}
