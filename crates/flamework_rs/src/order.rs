//! The order the TS transformer meets things in a file, which `flamework.build` records: new ids (its identifier
//! table, and the counters of `short` / `tiny` / `obfuscated` ids) and glob and path uses.
//!
//! roblox-ts lowers a call chain from its innermost call, so the hooks run inner first; the TS transformer visited
//! the outermost call first and its callee last (`transformUserMacro` builds the arguments, then transforms
//! `node.expression`). It also computed a class's metadata before transforming its members (and its decorators
//! after them), where the class hook runs after the members. Each record therefore carries a key that sorts it into
//! the TS transformer's order within the file; across files, the order is the program order (`finish` walks the
//! parts in it).

use rbxts_plugin::Cx;
use rbxts_plugin::ts::{Kind, Node, P};

use crate::metadata::get_symbol;
use crate::plugin::FwInstance;

/// A position in the TS transformer's walk of a file: one rank per level from the file down. Compared
/// lexicographically.
pub type OrderKey = Vec<(u32, u32)>;

/// The rank of a node's own record before its children (a class's metadata).
pub const BEFORE_CHILDREN: (u32, u32) = (0, 0);

fn offset(node: P<Node>) -> u32 {
    node.pos().source_offset().unwrap_or(0)
}

impl FwInstance {
    /// The key of a record made at `node`, with `own` its rank among the node's children: a generated macro argument
    /// is `(0, parameter index)`, ordered among the call's arguments; a class's metadata is [`BEFORE_CHILDREN`].
    pub(crate) fn order_key(&self, cx: &mut Cx<'_>, node: P<Node>, own: (u32, u32)) -> OrderKey {
        let mut key = vec![own];
        let mut child = node;
        while let Some(parent) = child.parent() {
            key.push(self.rank(cx, parent, child));
            if parent.kind() == Kind::SourceFile {
                break;
            }
            child = parent;
        }
        key.reverse();
        key
    }

    fn rank(&self, cx: &mut Cx<'_>, parent: P<Node>, child: P<Node>) -> (u32, u32) {
        match parent.kind() {
            Kind::CallExpression | Kind::NewExpression if self.is_macro_call(cx, parent) => {
                // The arguments in order (generated ones among them), then the callee.
                match parent.arguments().iter().position(|&a| a == child) {
                    Some(index) => (0, index as u32),
                    None => (1, offset(child)),
                }
            }
            Kind::ClassDeclaration if self.has_reflect(cx, parent) => {
                // `updateClass`: the metadata, then the members, then the decorators.
                if let Some(index) = parent.members().iter().position(|&m| m == child) {
                    (1, index as u32)
                } else if parent.decorators().contains(&child) {
                    (2, offset(child))
                } else {
                    (3, offset(child))
                }
            }
            _ => (0, offset(child)),
        }
    }

    /// The TS transformer takes the call through `transformUserMacro`: its callee is a user macro, and it is not
    /// `super(...)`.
    fn is_macro_call(&self, cx: &mut Cx<'_>, call: P<Node>) -> bool {
        let Some(callee) = call.expression() else { return false };
        if callee.kind() == Kind::SuperKeyword {
            return false;
        }
        match get_symbol(cx, callee) {
            Some(symbol) => self.is_user_macro(cx, symbol),
            None => false,
        }
    }

    fn has_reflect(&self, cx: &mut Cx<'_>, class: P<Node>) -> bool {
        let metadata = self.metadata(cx, class);
        crate::class_hook::has_reflect_metadata(self, cx, class, &metadata)
    }
}
