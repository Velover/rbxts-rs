//! Builds with TypeScript project references: roblox-ts's `ProjectBuild.build` over its `ProjectGraph`
//! ([`rbxts_project::graph`]).
//!
//! Each build makes the graph anew, then builds every project in build order, each with a [`BuildSession`] of its
//! own (its committed state, incremental state and watch facts are its own), and the root last:
//! - a solution config (only references) builds nothing;
//! - a project whose references, directly or not, failed is not built (`blocked`), and reports nothing itself;
//! - every project gets the graph's facts ([`GraphContext`]): its project data (options and Rojo file as the graph
//!   sets them), the reference paths an import of a referenced project's file names, and the other projects'
//!   outputs, which are never its assets;
//! - the reports are merged: the diagnostics of every project (sorted by the CLI), the files, warnings and errors.
//!
//! A program sees a referenced project through the declarations it emits (TypeScript's output redirects): those are
//! program files of their consumers, so a referenced project's declaration change re-transforms its importers there
//! by the usual invalidation rules, in the same build.
//!
//! rbxts-rs: deviations from roblox-ts, besides the per-project ones of [`BuildSession`]:
//! - a build with an outDir override (parity runs) builds only the root: the referenced projects' outputs are read
//!   as they are on disk;
//! - every buildable project is built every time (roblox-ts skips one whose inputs did not change in watch mode);
//!   an unchanged project transforms nothing and writes nothing, so only its program load is repeated;
//! - the runtime files are copied by each runtime project's commit (a successful build), as in a build without
//!   references, not before the build; a solution root of one deployment, which builds nothing, copies them after
//!   its projects' builds.

use std::collections::{HashMap, HashSet};

use rbxts_project::ProjectData;
use rbxts_project::graph::{GraphError, ProjectGraph, ProjectNode};

use super::outputs::GraphOutputs;
use super::session::{BuildSession, BuildSummary};
use crate::compile::{CompileOptions, CompileReport};

/// What the graph tells one project's builds.
#[derive(Clone, Debug)]
pub(crate) struct GraphContext {
    pub data: ProjectData,
    pub reference_paths: HashMap<String, String>,
    pub outputs: GraphOutputs,
}

impl GraphContext {
    fn of(node: &ProjectNode, outputs: &GraphOutputs) -> GraphContext {
        GraphContext {
            data: node.data.clone(),
            reference_paths: node.reference_paths.clone(),
            outputs: outputs.clone(),
        }
    }
}

/// The graph's outputs as every project sees them.
fn graph_outputs(graph: &ProjectGraph) -> GraphOutputs {
    let runtime = graph.runtime_projects();
    GraphOutputs {
        excluded_roots: graph.excluded_roots(),
        build_infos: graph.build_infos(),
        config_paths: graph.config_paths.clone(),
        rojo_files: graph.config_paths.iter().filter(|p| p.ends_with(".project.json")).cloned().collect(),
        include_paths: runtime.iter().map(|n| n.data.options.include_path.clone()).collect(),
    }
}

/// One build of the graph whose root is `root`'s project.
pub(super) fn build_graph(root: &mut BuildSession<'_>, touched: &[String]) -> (CompileReport, BuildSummary) {
    let tsconfig = root.tsconfig_path();
    let graph = match ProjectGraph::new(&tsconfig, &root.options.project_options, &root.options.cwd) {
        Ok(graph) => graph,
        Err(error) => {
            let mut report = CompileReport::default();
            match error {
                GraphError::Project(error) => report.fatal = error.messages,
                GraphError::Diagnostics(diagnostics) => report.diagnostics = diagnostics,
            }
            return (report, BuildSummary::default());
        }
    };
    root.graph_config_paths = graph.config_paths.clone();
    let outputs = graph_outputs(&graph);
    let mut reports: Vec<(CompileReport, BuildSummary)> = Vec::new();
    // the projects whose build failed or was blocked
    let mut failed: HashSet<String> = HashSet::new();
    let mut old: Vec<(String, BuildSession<'_>)> = std::mem::take(&mut root.references);
    let build_references = root.options.out_dir_override.is_none();
    let (references, root_node) = graph.nodes.split_at(graph.nodes.len() - 1);
    for node in references {
        if node.path_translator.is_none() {
            continue;
        }
        if node.dependencies.iter().any(|d| failed.contains(d)) {
            failed.insert(node.key.clone());
            continue;
        }
        if !build_references {
            continue;
        }
        let mut session = match old.iter().position(|(key, _)| *key == node.key) {
            Some(at) => old.swap_remove(at).1,
            None => {
                let options = CompileOptions {
                    tsconfig: node.data.tsconfig_path.clone(),
                    cwd: root.options.cwd.clone(),
                    project_options: root.options.project_options.clone(),
                    out_dir_override: None,
                    write_partial: root.options.write_partial,
                    no_write: root.options.no_write,
                    checkers: root.options.checkers,
                };
                let mut session = root.child(options);
                session.is_reference = true;
                session
            }
        };
        session.graph = Some(GraphContext::of(node, &outputs));
        let (report, summary) = session.build(touched);
        if report.has_errors() {
            failed.insert(node.key.clone());
        }
        reports.push((report, summary));
        root.references.push((node.key.clone(), session));
    }

    let node = &root_node[0];
    let blocked = node.dependencies.iter().any(|d| failed.contains(d));
    let root_built = node.path_translator.is_some() && !blocked;
    if root_built {
        root.graph = Some(GraphContext::of(node, &outputs));
        reports.push(root.build_project(touched));
    }
    let (mut report, mut summary) = merge(reports, root_built);
    if node.path_translator.is_none() && graph.shared_deployment && root.options.out_dir_override.is_none() {
        // a solution root of one deployment builds nothing, yet its include folder is the one its projects use:
        // roblox-ts copies its runtime files (`copyInclude` of the runtime projects) whatever the build does
        let copied = crate::project::copy_include::prepare_copy_include_of(&node.data)
            .and_then(|copy| copy.map_or_else(|| Ok(Vec::new()), |copy| copy.commit()));
        match copied {
            Ok(written) => summary.written.extend(written),
            Err(message) => report.fatal.push(message),
        }
    }
    (report, summary)
}

/// The reports of a graph's builds as one: the last one's (the root's, when it was built) with every other one's
/// diagnostics, files, warnings, errors and outputs added before its own.
fn merge(reports: Vec<(CompileReport, BuildSummary)>, root_built: bool) -> (CompileReport, BuildSummary) {
    let mut reports = reports.into_iter().rev();
    let (mut report, mut summary) = if root_built {
        reports.next().expect("the root's build")
    } else {
        (CompileReport::default(), BuildSummary::default())
    };
    for (other, other_summary) in reports {
        let mut other = other;
        prepend(&mut report.diagnostics, other.diagnostics);
        prepend(&mut report.warnings, other.warnings);
        prepend(&mut report.fatal, other.fatal);
        prepend(&mut report.files, std::mem::take(&mut other.files));
        prepend(&mut report.plugin_messages, other.plugin_messages);
        prepend(&mut report.artifacts_written, other.artifacts_written);
        prepend(&mut report.artifacts_deleted, other.artifacts_deleted);
        report.program_files += other.program_files;
        report.project_files += other.project_files;
        let t = &mut report.timings;
        let o = other.timings;
        t.config += o.config;
        t.program += o.program;
        t.setup += o.setup;
        t.invalidate += o.invalidate;
        t.check += o.check;
        t.plugin_setup += o.plugin_setup;
        t.transform += o.transform;
        t.plugin_finish += o.plugin_finish;
        t.render += o.render;
        t.load_check += o.load_check;
        t.load_checked += o.load_checked;
        t.write += o.write;
        summary.parsed += other_summary.parsed;
        prepend(&mut summary.rerendered, other_summary.rerendered);
        prepend(&mut summary.written, other_summary.written);
        prepend(&mut summary.deleted, other_summary.deleted);
        prepend(&mut summary.lost_state_files, other_summary.lost_state_files);
        prepend(&mut summary.setup_inputs_changed, other_summary.setup_inputs_changed);
    }
    (report, summary)
}

fn prepend<T>(to: &mut Vec<T>, mut items: Vec<T>) {
    items.append(to);
    *to = items;
}
