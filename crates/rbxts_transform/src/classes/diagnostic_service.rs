//! `classes/DiagnosticService.ts`: the diagnostics of one file's transform.
//!
//! rbxts-rs: roblox-ts keeps one static list for the whole build; here each file's `TransformState` owns one, and
//! the pipeline collects them in program order. `addSingleDiagnostic`'s once-per-code rule is per file.

use rustc_hash::FxHashSet;

use tsrs_ast::Diagnostic;
use tsrs_core::P;
use tsrs_diagnostics::Category;

/// `DiagnosticService`
#[derive(Default)]
pub struct DiagnosticService {
    pub diagnostics: Vec<P<Diagnostic>>,
    single_diagnostics: FxHashSet<String>,
    /// rbxts-rs: the node kinds (or forms) reported as not yet supported, in report order.
    pub unsupported: Vec<String>,
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

    /// `hasErrors`
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.category() == Category::Error)
    }
}
