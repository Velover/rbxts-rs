//! What nodes an instance wants (DESIGN §11.2). The compiler builds its dispatch tables from it once per instance,
//! so it never calls into a plugin for nodes the plugin did not ask for.

use tsrs_ast::Kind;

use crate::Tag;

/// Which classes reach [`crate::PluginInstance::class`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClassFilter {
    /// Every class declaration.
    All,
    /// Classes that have a decorator on the class or on a member.
    Decorated,
}

/// One [`Interest::calls_by_receiver`] registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReceiverInterest {
    /// Method names, compared first: most calls cost one string compare.
    pub methods: &'static [&'static str],
    /// The receiver's type must have this marker member.
    pub marker: &'static str,
    pub tag: Tag,
}

/// The interest tables of one instance, filled by [`crate::PluginInstance::interest`].
///
/// **Markers.** "The type has marker `m`" means `get_property_of_type(get_apparent_type(non_nullable(t)), m)` is
/// `Some`. The compiler may skip a marker entirely when no declaration in the program has a member of that name.
#[derive(Clone, Debug, Default)]
pub struct Interest {
    calls_by_symbol: bool,
    receiver_calls: Vec<ReceiverInterest>,
    callee_markers: Vec<(&'static str, Tag)>,
    access_markers: Vec<(&'static str, Tag)>,
    assignments_by_receiver_symbol: bool,
    classes: Option<ClassFilter>,
    expressions: Vec<(Kind, Tag)>,
    statements: Vec<(Kind, Tag)>,
}

impl Interest {
    /// An empty table. The compiler makes one per instance and passes it to `interest`.
    pub fn new() -> Interest {
        Interest::default()
    }

    // --- registration (plugins) ---

    /// Calls, `new` and decorator calls whose resolved callee symbol (aliases skipped) the instance classifies
    /// with [`crate::PluginInstance::classify_symbol`] (role [`crate::SymbolRole::Callee`]). A symbol classified
    /// `Some(tag)` sends its calls to [`crate::PluginInstance::call`] with that tag.
    pub fn calls_by_symbol(&mut self) {
        self.calls_by_symbol = true;
    }

    /// Method calls `obj.m(...)` with `m` in `methods` and `obj`'s type carrying `marker`.
    pub fn calls_by_receiver(
        &mut self,
        methods: &'static [&'static str],
        marker: &'static str,
        tag: Tag,
    ) {
        self.receiver_calls.push(ReceiverInterest {
            methods,
            marker,
            tag,
        });
    }

    /// Calls `x(...)` (and `obj.x(...)`) whose callee's type carries `marker`.
    pub fn calls_by_callee_marker(&mut self, marker: &'static str, tag: Tag) {
        self.callee_markers.push((marker, tag));
    }

    /// Property accesses `obj.k` / `obj["k"]` in any position whose receiver's type carries `marker`. They reach
    /// [`crate::PluginInstance::property_key`].
    ///
    /// Not dispatched yet: registering it fails the build until S2.
    pub fn accesses_by_marker(&mut self, marker: &'static str, tag: Tag) {
        self.access_markers.push((marker, tag));
    }

    /// Assignments, compound assignments, `++` / `--` and `delete` whose target is a property access; the
    /// property's symbol is classified with role [`crate::SymbolRole::AssignReceiver`].
    ///
    /// Not dispatched yet: registering it fails the build until S2.
    pub fn assignments_by_receiver_symbol(&mut self) {
        self.assignments_by_receiver_symbol = true;
    }

    /// Class declarations, for [`crate::PluginInstance::class`]. Registering `All` and `Decorated` gives `All`.
    pub fn classes(&mut self, filter: ClassFilter) {
        self.classes = match (self.classes, filter) {
            (Some(ClassFilter::All), _) | (_, ClassFilter::All) => Some(ClassFilter::All),
            _ => Some(ClassFilter::Decorated),
        };
    }

    /// Expressions of `kind`, for [`crate::PluginInstance::expression`]. Slow: the plugin is asked for each one it
    /// is offered. Expression hooks are asked before the call plugins, whatever the registration order.
    ///
    /// Not yet every expression of the kind (S2): calls inside a chain (the `f()` in `f().m()`) are offered only
    /// when a call plugin compiles the outer call itself, and accesses inside chains are not offered. One node may
    /// be offered twice (see the crate docs); answer the same way both times.
    pub fn expressions(&mut self, kind: Kind, tag: Tag) {
        self.expressions.push((kind, tag));
    }

    /// Every statement of `kind`, for [`crate::PluginInstance::statement`]. Slow: the plugin is asked for each.
    pub fn statements(&mut self, kind: Kind, tag: Tag) {
        self.statements.push((kind, tag));
    }

    // --- reading (the compiler) ---

    /// Whether [`Interest::calls_by_symbol`] was registered.
    pub fn wants_calls_by_symbol(&self) -> bool {
        self.calls_by_symbol
    }

    /// The [`Interest::calls_by_receiver`] registrations, in order.
    pub fn receiver_calls(&self) -> &[ReceiverInterest] {
        &self.receiver_calls
    }

    /// The [`Interest::calls_by_callee_marker`] registrations, in order.
    pub fn callee_markers(&self) -> &[(&'static str, Tag)] {
        &self.callee_markers
    }

    /// The [`Interest::accesses_by_marker`] registrations, in order.
    pub fn access_markers(&self) -> &[(&'static str, Tag)] {
        &self.access_markers
    }

    /// Whether [`Interest::assignments_by_receiver_symbol`] was registered.
    pub fn wants_assignments_by_receiver_symbol(&self) -> bool {
        self.assignments_by_receiver_symbol
    }

    /// The class filter, if [`Interest::classes`] was registered.
    pub fn class_filter(&self) -> Option<ClassFilter> {
        self.classes
    }

    /// The [`Interest::expressions`] registrations, in order.
    pub fn expression_kinds(&self) -> &[(Kind, Tag)] {
        &self.expressions
    }

    /// The [`Interest::statements`] registrations, in order.
    pub fn statement_kinds(&self) -> &[(Kind, Tag)] {
        &self.statements
    }

    /// Nothing registered: the compiler can leave the instance out of every dispatch table (broadcast hooks
    /// `begin_file` / `end_file` still run).
    pub fn is_empty(&self) -> bool {
        !self.calls_by_symbol
            && self.receiver_calls.is_empty()
            && self.callee_markers.is_empty()
            && self.access_markers.is_empty()
            && !self.assignments_by_receiver_symbol
            && self.classes.is_none()
            && self.expressions.is_empty()
            && self.statements.is_empty()
    }
}
