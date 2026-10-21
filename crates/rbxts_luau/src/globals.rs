//! `luau.globals` and `luau.strings`: the Luau globals and string constants roblox-ts emits, as fresh nodes.
//!
//! Rust keywords get a trailing underscore or a raw name: `self_()`, `super_()`, `r#type()`, `r#typeof()`,
//! `coroutine::r#yield()`, `string::r#match()`, `table::r#move()`.

use crate::ast::Expr;
use crate::create::{id, property, string};

macro_rules! ids {
    ($($(#[$meta:meta])* $fn_name:ident = $name:literal),* $(,)?) => {
        $( $(#[$meta])* #[doc = concat!("`", $name, "`")] pub fn $fn_name() -> Expr { id($name) } )*
    };
}

macro_rules! members {
    ($module:ident: $($fn_name:ident = $name:literal),* $(,)?) => {
        #[doc = concat!("Members of `", stringify!($module), "`.")]
        pub mod $module {
            use super::*;
            $( #[doc = concat!("`", stringify!($module), ".", $name, "`")] pub fn $fn_name() -> Expr {
                property(id(stringify!($module)), $name)
            } )*
        }
    };
}

ids!(
    _g = "_G",
    ts = "TS",
    assert = "assert",
    bit32 = "bit32",
    error = "error",
    exports = "exports",
    getmetatable = "getmetatable",
    ipairs = "ipairs",
    next = "next",
    pairs = "pairs",
    pcall = "pcall",
    require = "require",
    script = "script",
    select = "select",
    self_ = "self",
    setmetatable = "setmetatable",
    super_ = "super",
    tostring = "tostring",
    r#type = "type",
    r#typeof = "typeof",
    unpack = "unpack",
    // roblox
    game = "game",
);

members!(coroutine: r#yield = "yield");
members!(string:
    byte = "byte", find = "find", format = "format", gmatch = "gmatch", gsub = "gsub", lower = "lower",
    r#match = "match", rep = "rep", reverse = "reverse", split = "split", sub = "sub", upper = "upper",
);
members!(table:
    clear = "clear", clone = "clone", concat = "concat", create = "create", find = "find", freeze = "freeze",
    insert = "insert", isfrozen = "isfrozen", maxn = "maxn", r#move = "move", pack = "pack", remove = "remove",
    sort = "sort", unpack = "unpack",
);
members!(utf8: charpattern = "charpattern", codes = "codes");
members!(math: min = "min");

/// `luau.strings`: string literals roblox-ts uses often.
pub mod strings {
    use super::*;

    macro_rules! strings {
        ($($fn_name:ident = $value:literal),* $(,)?) => {
            $( #[doc = concat!("`\"", $value, "\"`")] pub fn $fn_name() -> Expr { string($value) } )*
        };
    }

    strings!(
        // metamethods
        __index = "__index",
        __tostring = "__tostring",
        __mode = "__mode",
        k = "k",
        // types
        number = "number",
        table = "table",
        // opcall
        success = "success",
        value = "value",
        error = "error",
        // ReadonlyArray.join()
        comma_space = ", ",
    );
}
