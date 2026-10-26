//! A project file's transform diagnostics kept in the incremental state ([`DiskDiagnostic`]) and made again in a
//! new process for a file it does not transform, so an incremental build reports what a clean build would.
//!
//! Only roblox-ts's and plugins' diagnostics are kept: ad hoc messages (code -1) without a message chain. A file
//! with any other diagnostic is marked so the next process transforms it again ([`to_disk`]'s answer).

use std::sync::{Mutex, PoisonError};

use rustc_hash::FxHashMap;
use tsrs_ast::{Diagnostic, DiagnosticExt, SourceFile, new_diagnostic};
use tsrs_compiler::Program;
use tsrs_core::{P, TextRange};
use tsrs_diagnostics::{Category, new_ad_hoc_message};

use super::state::DiskDiagnostic;

fn category_number(category: Category) -> u8 {
    match category {
        Category::Warning => 0,
        Category::Error => 1,
        Category::Suggestion => 2,
        Category::Message => 3,
    }
}

fn category_of(number: u8) -> Category {
    match number {
        0 => Category::Warning,
        2 => Category::Suggestion,
        3 => Category::Message,
        _ => Category::Error,
    }
}

/// A kept diagnostic's span and text, `None` for one that can't be kept (a code of its own, a message chain).
fn plain(d: P<Diagnostic>) -> Option<(u32, u32, String)> {
    if d.code() != -1 || !d.message_chain().is_empty() {
        return None;
    }
    let loc = d.loc()?;
    let pos = loc.pos().expect_source("a transform diagnostic has a source position");
    Some((pos, pos + loc.len()?, d.localize()))
}

/// `owner`'s diagnostics as the incremental state keeps them (`plugin`: which of them plugins reported), and whether
/// one of them could not be kept.
pub fn to_disk(diagnostics: &[P<Diagnostic>], plugin: &[P<Diagnostic>], owner: P<SourceFile>) -> (Vec<DiskDiagnostic>, bool) {
    let owner = owner.get().file_name();
    let mut kept = Vec::with_capacity(diagnostics.len());
    for &d in diagnostics {
        let Some((pos, end, text)) = plain(d) else { return (Vec::new(), true) };
        let mut related = Vec::new();
        for &r in d.related_information() {
            let Some((pos, end, text)) = plain(r) else { return (Vec::new(), true) };
            related.push((r.file().map(|f| f.get().file_name().to_owned()), pos, end, category_number(r.category()), text));
        }
        kept.push(DiskDiagnostic {
            file: d.file().map(|f| f.get().file_name()).filter(|&name| name != owner).map(str::to_owned),
            pos,
            end,
            category: category_number(d.category()),
            text,
            source: crate::plugins::diagnostic_source(d).map(str::to_owned),
            plugin: plugin.contains(&d),
            related,
        });
    }
    (kept, false)
}

/// The sources of replayed plugin diagnostics, by the address of their ad hoc message (as the plugin backend keeps
/// its own diagnostics').
static SOURCES: Mutex<Option<FxHashMap<usize, &'static str>>> = Mutex::new(None);

/// The source of a diagnostic [`from_disk`] made, `None` for any other.
pub fn source(diagnostic: P<Diagnostic>) -> Option<&'static str> {
    let address = std::ptr::from_ref(diagnostic.message()?) as usize;
    SOURCES.lock().unwrap_or_else(PoisonError::into_inner).as_ref()?.get(&address).copied()
}

/// A diagnostic `file` kept, made again in `program` (one at another file, and related information, in the program's
/// files by name). `None` when the program no longer has the other file.
pub fn from_disk(kept: &DiskDiagnostic, file: P<SourceFile>, program: &Program) -> Option<P<Diagnostic>> {
    let make = |file: Option<P<SourceFile>>, pos: u32, end: u32, category: u8, text: &str| {
        let d = new_diagnostic(file, TextRange::new(pos, end), new_ad_hoc_message(text), &[]);
        if category_of(category) != Category::Error {
            d.set_category(category_of(category));
        }
        d
    };
    let file = match &kept.file {
        Some(name) => program.get_source_file(name)?,
        None => file,
    };
    let d = make(Some(file), kept.pos, kept.end, kept.category, &kept.text);
    if let Some(source) = &kept.source
        && let Some(message) = d.message()
    {
        let source: &'static str = Box::leak(source.clone().into_boxed_str());
        SOURCES
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(FxHashMap::default)
            .insert(std::ptr::from_ref(message) as usize, source);
    }
    if !kept.related.is_empty() {
        let related: Vec<P<Diagnostic>> = kept
            .related
            .iter()
            .map(|(name, pos, end, category, text)| {
                make(name.as_deref().and_then(|n| program.get_source_file(n)), *pos, *end, *category, text)
            })
            .collect();
        d.set_related_info(&related);
    }
    Some(d)
}
