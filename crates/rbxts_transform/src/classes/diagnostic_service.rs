//! `classes/DiagnosticService.ts`: the diagnostics of one file's transform.
//!
//! rbxts-rs: roblox-ts keeps one static list for the whole build; here each file's `TransformState` owns one, and
//! the pipeline collects them in transform order. `addSingleDiagnostic`'s once-per-code rule is per file.
//!
//! `addDiagnosticWithCache` (once per compilation and symbol) does not depend on the checker count: its caches are
//! keyed by [`ReportOnce`], each file records the keys of what it reported, and the build drops a file's diagnostic
//! whose key an earlier file (transform order) reported ([`drop_reported_earlier`]).

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

/// rbxts-rs: the key of a once-per-compilation diagnostic, the same on every checker and whatever each checker
/// checked before. A symbol with declarations is keyed by its first declaration and its name, whether it is the
/// binder's symbol or one a checker made (merged or instantiated: `Transient`, a different object on each checker).
/// The two must share a key: `instantiateSymbol` answers a property of an instantiation (`Holder<string>.value`) with
/// the declared symbol when that property's type is already resolved and has no type variables, else with a new
/// transient one, so which of the two a checker gives depends on what it resolved before. A symbol without
/// declarations has no identity across checkers: a binder one is its own key, a transient one is reported once per
/// file.
///
/// Against stock roblox-ts (one checker, keyed by symbol object): the same, except that two distinct symbols with
/// one first declaration and name report once here, where stock may report each. That needs a property whose type
/// does depend on the instantiation (`value: T` with `T = any` in two instantiations), or a mapped type's property
/// next to its source property; and stock's own answer there depends on its checker's history too.
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
        let key = if let Some(&declaration) = symbol.declarations().first() {
            OnceKey::Declared(declaration, symbol.name())
        } else if !symbol.flags().intersects(SymbolFlags::Transient) {
            OnceKey::Symbol(symbol)
        } else {
            OnceKey::InFile(symbol, file)
        };
        ReportOnce { cache, key }
    }
}

/// rbxts-rs: `files` in transform order, each as its diagnostics and the once-per-compilation ones among them. Drops
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
