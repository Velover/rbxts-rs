//! `classes/DiagnosticService.ts`: the diagnostics of one file's transform.
//!
//! rbxts-rs: roblox-ts keeps one static list for the whole build; here each file's `TransformState` owns one, and
//! the pipeline collects them in program order. `addSingleDiagnostic`'s once-per-code rule is per file.
//!
//! `addDiagnosticWithCache` (once per compilation and symbol) does not depend on the checker count: its caches are
//! keyed by [`ReportOnce`], each file records the keys of what it reported, and the build drops a file's diagnostic
//! whose key an earlier file (program order) reported ([`drop_reported_earlier`]).

use rustc_hash::FxHashSet;

use tsrs_ast::{Diagnostic, Node, SourceFile, Symbol, SymbolFlags};
use tsrs_core::P;
use tsrs_diagnostics::Category;

/// `DiagnosticService`
#[derive(Default)]
pub struct DiagnosticService {
    pub diagnostics: Vec<P<Diagnostic>>,
    single_diagnostics: FxHashSet<String>,
    /// rbxts-rs: the node kinds (or forms) reported as not yet supported, in report order.
    pub unsupported: Vec<String>,
    /// rbxts-rs: the diagnostics reported once per compilation, with their keys ([`drop_reported_earlier`]).
    pub once: Vec<(ReportOnce, P<Diagnostic>)>,
}

impl DiagnosticService {
    /// `addDiagnostic`
    pub fn add_diagnostic(&mut self, diagnostic: P<Diagnostic>) {
        self.diagnostics.push(diagnostic);
    }

    /// `addSingleDiagnostic`: only the first diagnostic with this message.
    pub fn add_single_diagnostic(&mut self, diagnostic: P<Diagnostic>) {
        let key = diagnostic.message().map(|m| m.text().to_owned()).unwrap_or_default();
        if self.single_diagnostics.insert(key) {
            self.add_diagnostic(diagnostic);
        }
    }

    /// `addDiagnosticWithCache`'s report, once its cache took `key`.
    pub fn add_diagnostic_once(&mut self, key: ReportOnce, diagnostic: P<Diagnostic>) {
        self.once.push((key, diagnostic));
        self.add_diagnostic(diagnostic);
    }

    /// `hasErrors`
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.category() == Category::Error)
    }
}

/// rbxts-rs: which `addDiagnosticWithCache` cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OnceCache {
    /// `isReportedByNoAnyCache`
    NoAny,
    /// `isReportedByMultipleDefinitionsCache`
    MultipleDefinitions,
}

/// rbxts-rs: the key of a once-per-compilation diagnostic, the same on every checker. A binder symbol is shared by
/// the checkers, so it is its own key. A symbol a checker made (merged or instantiated: `Transient`) is a different
/// object on each checker, so it is keyed by its first declaration and its name; one without declarations has no
/// identity across checkers and is reported once per file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReportOnce {
    cache: OnceCache,
    key: OnceKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum OnceKey {
    Symbol(P<Symbol>),
    Declared(P<Node>, &'static str),
    InFile(P<Symbol>, P<SourceFile>),
}

impl ReportOnce {
    /// The key of `symbol` in `cache`, reported in `file`.
    pub fn new(cache: OnceCache, symbol: P<Symbol>, file: P<SourceFile>) -> ReportOnce {
        let key = if !symbol.flags().intersects(SymbolFlags::Transient) {
            OnceKey::Symbol(symbol)
        } else if let Some(&declaration) = symbol.declarations().first() {
            OnceKey::Declared(declaration, symbol.name())
        } else {
            OnceKey::InFile(symbol, file)
        };
        ReportOnce { cache, key }
    }
}

/// rbxts-rs: `files` in program order, each as its diagnostics and the once-per-compilation ones among them. Drops
/// each diagnostic whose key an earlier file reported (within a file, the transform's cache kept the first).
pub fn drop_reported_earlier<'a>(
    files: impl Iterator<Item = (&'a mut Vec<P<Diagnostic>>, &'a [(ReportOnce, P<Diagnostic>)])>,
) {
    let mut reported: FxHashSet<ReportOnce> = FxHashSet::default();
    for (diagnostics, once) in files {
        let dropped: FxHashSet<P<Diagnostic>> =
            once.iter().filter(|(key, _)| !reported.insert(*key)).map(|&(_, diagnostic)| diagnostic).collect();
        if !dropped.is_empty() {
            diagnostics.retain(|d| !dropped.contains(d));
        }
    }
}
