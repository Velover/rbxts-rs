//! roblox-ts's macro tables (`macros/*.ts`). In T1 only the names are here: the macro manager resolves their
//! symbols, and the transforms come with the macro area of T2 (each `None` is one macro to port).

pub mod types;

use types::{CallMacro, ConstructorMacro, IdentifierMacro, PropertyCallMacro};

/// `IDENTIFIER_MACROS`
pub const IDENTIFIER_MACROS: &[(&str, Option<IdentifierMacro>)] = &[("Promise", None)];

/// `CALL_MACROS`
pub const CALL_MACROS: &[(&str, Option<CallMacro>)] = &[
    ("assert", None),
    ("typeOf", None),
    ("typeIs", None),
    ("classIs", None),
    ("identity", None),
    ("$range", None),
    ("$tuple", None),
    ("$getModuleTree", None),
];

/// `CONSTRUCTOR_MACROS`, by constructor interface.
pub const CONSTRUCTOR_MACROS: &[(&str, Option<ConstructorMacro>)] = &[
    ("ArrayConstructor", None),
    ("SetConstructor", None),
    ("MapConstructor", None),
    ("WeakSetConstructor", None),
    ("WeakMapConstructor", None),
    ("ReadonlyMapConstructor", None),
    ("ReadonlySetConstructor", None),
];

type PropertyCallMacros = &'static [(&'static str, Option<PropertyCallMacro>)];

/// `PROPERTY_CALL_MACROS`, by interface, in roblox-ts's order.
pub const PROPERTY_CALL_MACROS: &[(&str, PropertyCallMacros)] = &[
    ("CFrame", &[("add", None), ("sub", None), ("mul", None)]),
    ("UDim", &[("add", None), ("sub", None)]),
    ("UDim2", &[("add", None), ("sub", None)]),
    ("Vector2", &[("add", None), ("sub", None), ("mul", None), ("div", None), ("idiv", None)]),
    ("Vector2int16", &[("add", None), ("sub", None), ("mul", None), ("div", None)]),
    ("Vector3", &[("add", None), ("sub", None), ("mul", None), ("div", None), ("idiv", None)]),
    ("Vector3int16", &[("add", None), ("sub", None), ("mul", None), ("div", None)]),
    ("vector", &[("add", None), ("sub", None), ("mul", None), ("div", None), ("idiv", None)]),
    ("Number", &[("idiv", None)]),
    (
        "String",
        &[
            ("size", None),
            ("byte", None),
            ("find", None),
            ("format", None),
            ("gmatch", None),
            ("gsub", None),
            ("lower", None),
            ("match", None),
            ("rep", None),
            ("reverse", None),
            ("split", None),
            ("sub", None),
            ("upper", None),
        ],
    ),
    ("ArrayLike", &[("size", None)]),
    (
        "ReadonlyArray",
        &[
            ("isEmpty", None),
            ("join", None),
            ("move", None),
            ("includes", None),
            ("indexOf", None),
            ("every", None),
            ("some", None),
            ("forEach", None),
            ("map", None),
            ("mapFiltered", None),
            ("filterUndefined", None),
            ("filter", None),
            ("reduce", None),
            ("find", None),
            ("findIndex", None),
        ],
    ),
    (
        "Array",
        &[
            ("push", None),
            ("pop", None),
            ("shift", None),
            ("unshift", None),
            ("insert", None),
            ("remove", None),
            ("unorderedRemove", None),
            ("sort", None),
            ("clear", None),
        ],
    ),
    ("ReadonlySet", &[("isEmpty", None), ("size", None), ("has", None), ("forEach", None)]),
    ("Set", &[("delete", None), ("clear", None), ("add", None)]),
    ("ReadonlyMap", &[("isEmpty", None), ("size", None), ("has", None), ("forEach", None), ("get", None)]),
    ("Map", &[("delete", None), ("clear", None), ("set", None)]),
    ("Promise", &[("then", None)]),
];
