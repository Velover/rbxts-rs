//! `identity`: claims every call, `new` and decorator by symbol and lowers it as roblox-ts would, with
//! `compile_expr(call.node)` ([`IdentityMode::Expr`]) or `compile_call(Callee::Original, all Original)`
//! ([`IdentityMode::Call`]). Its output must equal a build without plugins: a check of the lowering primitives on a
//! whole corpus (`rbxts-bench build --plugin identity-expr --parity …`). Calls of the compiler-types (roblox-ts's
//! macros) and of `@rbxts/types` (where some of them are declared) are declined, as `compile_call` cannot pass
//! argument slots to a macro; call shapes S1 does not offer (`o.m?.()`, `super.m()`) are reported as internal errors
//! by the compiler.

use std::sync::Arc;

use rbxts_plugin::ts::{Kind, P, Symbol};
use rbxts_plugin::{
    Arg, CallKind, CallSite, Callee, CheckerInfo, Claim, Cx, Fatal, FileIndex, FinishCx, Interest, LuauExpr, PResult,
    Plugin, PluginInfo, PluginInstance, SetupCx, SymbolRole, Tag,
};

/// How `identity` lowers what it claims.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityMode {
    /// `compile_expr(call.node)` (a decorator: `compile_expr` of its expression).
    Expr,
    /// `compile_call(Callee::Original, every argument Original)` (a decorator without a call: `compile_expr`).
    Call,
}

pub struct Identity(pub IdentityMode);

pub struct IdentityInstance(IdentityMode);

impl Plugin for Identity {
    type Shared = IdentityMode;
    type State = ();
    type FilePart = ();
    type Instance = IdentityInstance;

    fn info(&self) -> PluginInfo {
        PluginInfo { name: "identity", version: "0.1.0", tsconfig_transform: None }
    }

    fn setup(&self, _: &mut SetupCx<'_>, _: &mut ()) -> Result<Arc<IdentityMode>, Fatal> {
        Ok(Arc::new(self.0))
    }

    fn instance(&self, shared: Arc<IdentityMode>, _: CheckerInfo) -> IdentityInstance {
        IdentityInstance(*shared)
    }

    fn finish(&self, _: &IdentityMode, _: &mut (), _: &[(FileIndex, &())], _: &mut FinishCx<'_>) -> Result<(), Fatal> {
        Ok(())
    }
}

impl PluginInstance for IdentityInstance {
    type FilePart = ();

    fn interest(&self, interest: &mut Interest) {
        interest.calls_by_symbol();
    }

    fn classify_symbol(&self, cx: &mut Cx<'_>, symbol: P<Symbol>, role: SymbolRole) -> Option<Tag> {
        // roblox-ts's macros are declared in these two packages (`String.gmatch`, `Vector3.mul`, …)
        let macros = matches!(
            cx.declaring_package(symbol).and_then(|p| p.name.clone()).as_deref(),
            Some("@rbxts/compiler-types" | "@rbxts/types")
        );
        (role == SymbolRole::Callee && !macros).then_some(Tag(0))
    }

    fn call(&self, cx: &mut Cx<'_>, call: &CallSite, _: Tag) -> PResult<Claim<LuauExpr>> {
        let value = if call.kind == CallKind::Decorator {
            let expression = call.node.expression().expect("a decorator has an expression");
            if self.0 == IdentityMode::Call && expression.kind() == Kind::CallExpression {
                cx.compile_call(call, Callee::Original, call.args.iter().map(|&a| Arg::Original(a)).collect())?
            } else {
                cx.compile_expr(expression)?
            }
        } else if self.0 == IdentityMode::Expr {
            cx.compile_expr(call.node)?
        } else {
            cx.compile_call(call, Callee::Original, call.args.iter().map(|&a| Arg::Original(a)).collect())?
        };
        Ok(Claim::Take(value))
    }

    fn end_file(&self, _: &mut Cx<'_>) -> PResult<()> {
        Ok(())
    }
}
