//! The Luau syntax tree (`LuauAST/types` of `@roblox-ts/luau-ast`).
//!
//! Differences from the TypeScript tree, none of which changes what is rendered:
//! - **No `parent`.** The renderer passes what it needs from the parent down while it walks (parentheses,
//!   long-bracket spacing, block edges for temporary identifiers).
//! - **`luau.List` is a `Vec`.** A field typed `X | List<X>` in TypeScript is a `Vec` here: one element renders the
//!   same as the single node.
//! - **Node kinds are enum variants.** `luau.isX` guards become `match`; the semantic guards (`is_simple`,
//!   `is_indexable`, …) are methods.
//! - **Late strings** ([`Expr::LateString`]) are new: string literals whose text is known only at render time.

/// An expression. The first seven variants are TypeScript's `IndexableExpression`s: the ones that can be called,
/// indexed or have a method called on them without parentheses (see [`Expr::is_indexable`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    // indexable expressions
    Identifier(Identifier),
    TemporaryIdentifier(TemporaryIdentifier),
    ComputedIndex(Box<ComputedIndexExpression>),
    PropertyAccess(Box<PropertyAccessExpression>),
    Call(Box<CallExpression>),
    MethodCall(Box<MethodCallExpression>),
    Parenthesized(Box<Expr>),

    // expressions
    /// A placeholder for "no expression"; rendering it is a compiler bug.
    None,
    Nil,
    False,
    True,
    Number(NumberLiteral),
    String(StringLiteral),
    /// A string literal whose value is looked up by key in the [`crate::LateStrings`] given to the renderer
    /// (DESIGN §4 step 5). It renders and behaves exactly like a [`StringLiteral`] with that value and no quote.
    LateString(LateString),
    /// `...`
    VarArgs,
    Function(Box<FunctionExpression>),
    Binary(Box<BinaryExpression>),
    Unary(Box<UnaryExpression>),
    If(Box<IfExpression>),
    InterpolatedString(InterpolatedString),
    /// `{ a, b }`
    Array(Vec<Expr>),
    /// `{ k = v, [k] = v }`, one field per line.
    Map(Vec<MapField>),
    /// `{ k = true, [k] = true }`, one member per line.
    Set(Vec<Expr>),
    /// Positional and keyed fields, one per line.
    MixedTable(Vec<TableField>),
}

/// A Luau identifier. Rendering asserts that `name` is a valid identifier ([`crate::is_valid_identifier`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Identifier {
    pub name: String,
}

/// A local whose final name is chosen at render time, unique within its scope: `_name`, `_name_1`, … (or `_`,
/// `_1`, … for an empty name).
///
/// Identity is `id`: every copy of a temporary identifier with the same id renders as the same name, as every clone
/// of the TypeScript node shares its id. Ids are unique per file (see [`crate::TempIds`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TemporaryIdentifier {
    pub name: String,
    pub id: u32,
}

/// `expression[index]`, or `expression.name` when the index is a string literal that is a valid identifier.
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedIndexExpression {
    /// Should be indexable ([`Expr::is_indexable`]).
    pub expression: Expr,
    pub index: Expr,
}

/// `expression.name`, or `expression["name"]` when `name` is not a valid identifier (the name is not escaped).
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyAccessExpression {
    /// Should be indexable ([`Expr::is_indexable`]).
    pub expression: Expr,
    pub name: String,
}

/// `expression(args)`
#[derive(Clone, Debug, PartialEq)]
pub struct CallExpression {
    /// Should be indexable ([`Expr::is_indexable`]).
    pub expression: Expr,
    pub args: Vec<Expr>,
}

/// `expression:name(args)`. Rendering asserts that `name` is a valid identifier.
#[derive(Clone, Debug, PartialEq)]
pub struct MethodCallExpression {
    pub name: String,
    /// Should be indexable ([`Expr::is_indexable`]).
    pub expression: Expr,
    pub args: Vec<Expr>,
}

/// A number literal, as source text. Text that is not a valid Luau number literal (`0o17`, `Infinity`) is
/// rendered as JavaScript's `String(Number(text))`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NumberLiteral {
    pub value: String,
}

/// A string literal. `value` is the text between the quotes, already escaped: the renderer only picks the
/// delimiter (`"`, `'`, then a long bracket `[[…]]` when the text has both quotes or a newline).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StringLiteral {
    pub value: String,
    /// An explicit delimiter; `value` must then be escaped for it.
    pub quote: Option<Quote>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quote {
    /// `"`
    Double,
    /// `'`
    Single,
}

impl Quote {
    pub fn as_char(self) -> char {
        match self {
            Quote::Double => '"',
            Quote::Single => '\'',
        }
    }
}

/// See [`Expr::LateString`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LateString {
    pub key: String,
}

/// `function(parameters) statements end`
#[derive(Clone, Debug, PartialEq)]
pub struct FunctionExpression {
    pub parameters: Vec<AnyIdentifier>,
    /// Adds `...` after the parameters.
    pub has_dot_dot_dot: bool,
    pub statements: Vec<Stmt>,
}

/// `left operator right`, parenthesized when the parent binds tighter.
#[derive(Clone, Debug, PartialEq)]
pub struct BinaryExpression {
    pub left: Expr,
    pub operator: BinaryOperator,
    pub right: Expr,
}

/// `operator expression`, parenthesized when the parent binds tighter.
#[derive(Clone, Debug, PartialEq)]
pub struct UnaryExpression {
    pub operator: UnaryOperator,
    pub expression: Expr,
}

/// `if condition then expression else alternative`; an [`IfExpression`] alternative renders as `elseif`.
#[derive(Clone, Debug, PartialEq)]
pub struct IfExpression {
    pub condition: Expr,
    pub expression: Expr,
    pub alternative: Expr,
}

/// `` `text{expression}text` ``
#[derive(Clone, Debug, PartialEq)]
pub struct InterpolatedString {
    pub parts: Vec<InterpolatedStringPart>,
}

/// A part of an [`InterpolatedString`].
#[derive(Clone, Debug, PartialEq)]
pub enum InterpolatedStringPart {
    /// Literal text, escaped as for a string literal; the renderer escapes `{`, `}` and line breaks.
    Text(String),
    Expression(Expr),
}

/// A keyed field of a [`Expr::Map`] or [`Expr::MixedTable`]: `index = value`, or `[index] = value`.
#[derive(Clone, Debug, PartialEq)]
pub struct MapField {
    pub index: Expr,
    pub value: Expr,
}

/// A field of a [`Expr::MixedTable`].
#[derive(Clone, Debug, PartialEq)]
pub enum TableField {
    Value(Expr),
    Field(MapField),
}

/// A statement.
#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Assignment(Box<Assignment>),
    Break,
    /// A call statement; the expression is an [`Expr::Call`] or an [`Expr::MethodCall`].
    Call(Expr),
    Continue,
    Do(DoStatement),
    While(Box<WhileStatement>),
    Repeat(Box<RepeatStatement>),
    If(Box<IfStatement>),
    NumericFor(Box<NumericForStatement>),
    For(Box<ForStatement>),
    FunctionDeclaration(Box<FunctionDeclaration>),
    MethodDeclaration(Box<MethodDeclaration>),
    VariableDeclaration(Box<VariableDeclaration>),
    Return(ReturnStatement),
    /// `--text`, or a `--[[ ]]` block when the text has several lines.
    Comment(String),
}

/// `left operator right`, with comma-separated lists on both sides.
#[derive(Clone, Debug, PartialEq)]
pub struct Assignment {
    /// Writable expressions (identifiers, property accesses, computed indexes); not empty.
    pub left: Vec<Expr>,
    pub operator: AssignmentOperator,
    /// Not empty.
    pub right: Vec<Expr>,
}

/// `do statements end`
#[derive(Clone, Debug, PartialEq)]
pub struct DoStatement {
    pub statements: Vec<Stmt>,
}

/// `while condition do statements end`
#[derive(Clone, Debug, PartialEq)]
pub struct WhileStatement {
    pub condition: Expr,
    pub statements: Vec<Stmt>,
}

/// `repeat statements until condition`
#[derive(Clone, Debug, PartialEq)]
pub struct RepeatStatement {
    pub condition: Expr,
    pub statements: Vec<Stmt>,
}

/// `if condition then statements [elseif … | else …] end`
#[derive(Clone, Debug, PartialEq)]
pub struct IfStatement {
    pub condition: Expr,
    pub statements: Vec<Stmt>,
    pub else_body: ElseBody,
}

/// What follows the `then` block of an [`IfStatement`].
#[derive(Clone, Debug, PartialEq)]
pub enum ElseBody {
    /// `elseif …`
    If(Box<IfStatement>),
    /// `else statements`; nothing when empty.
    Block(Vec<Stmt>),
}

impl Default for ElseBody {
    fn default() -> Self {
        ElseBody::Block(Vec::new())
    }
}

/// `for id = start, end[, step] do statements end`; a literal step of 1 is left out.
#[derive(Clone, Debug, PartialEq)]
pub struct NumericForStatement {
    pub id: AnyIdentifier,
    pub start: Expr,
    pub end: Expr,
    pub step: Option<Expr>,
    pub statements: Vec<Stmt>,
}

/// `for ids in expression do statements end`; no ids render as `_`.
#[derive(Clone, Debug, PartialEq)]
pub struct ForStatement {
    pub ids: Vec<AnyIdentifier>,
    pub expression: Expr,
    pub statements: Vec<Stmt>,
}

/// `[local ]function name(parameters) statements end`
#[derive(Clone, Debug, PartialEq)]
pub struct FunctionDeclaration {
    /// `local function`; `name` must then be an identifier.
    pub localize: bool,
    /// An identifier, temporary identifier or property access.
    pub name: Expr,
    pub parameters: Vec<AnyIdentifier>,
    pub has_dot_dot_dot: bool,
    pub statements: Vec<Stmt>,
}

/// `function expression:name(parameters) statements end`
#[derive(Clone, Debug, PartialEq)]
pub struct MethodDeclaration {
    /// Should be indexable ([`Expr::is_indexable`]).
    pub expression: Expr,
    pub name: String,
    pub parameters: Vec<AnyIdentifier>,
    pub has_dot_dot_dot: bool,
    pub statements: Vec<Stmt>,
}

/// `local left[ = right]`
#[derive(Clone, Debug, PartialEq)]
pub struct VariableDeclaration {
    /// Not empty.
    pub left: Vec<AnyIdentifier>,
    /// Empty for `local a` (no initializer).
    pub right: Vec<Expr>,
}

/// `return expressions` (comma-separated).
#[derive(Clone, Debug, PartialEq)]
pub struct ReturnStatement {
    pub expressions: Vec<Expr>,
}

/// An identifier or a temporary identifier: what can be declared (parameters, locals, loop variables).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AnyIdentifier {
    Identifier(Identifier),
    Temporary(TemporaryIdentifier),
}

impl From<AnyIdentifier> for Expr {
    fn from(id: AnyIdentifier) -> Expr {
        match id {
            AnyIdentifier::Identifier(id) => Expr::Identifier(id),
            AnyIdentifier::Temporary(id) => Expr::TemporaryIdentifier(id),
        }
    }
}

impl From<Identifier> for Expr {
    fn from(id: Identifier) -> Expr {
        Expr::Identifier(id)
    }
}

impl From<TemporaryIdentifier> for Expr {
    fn from(id: TemporaryIdentifier) -> Expr {
        Expr::TemporaryIdentifier(id)
    }
}

impl From<Identifier> for AnyIdentifier {
    fn from(id: Identifier) -> AnyIdentifier {
        AnyIdentifier::Identifier(id)
    }
}

impl From<TemporaryIdentifier> for AnyIdentifier {
    fn from(id: TemporaryIdentifier) -> AnyIdentifier {
        AnyIdentifier::Temporary(id)
    }
}

impl TryFrom<Expr> for AnyIdentifier {
    type Error = Expr;

    fn try_from(expr: Expr) -> Result<AnyIdentifier, Expr> {
        match expr {
            Expr::Identifier(id) => Ok(AnyIdentifier::Identifier(id)),
            Expr::TemporaryIdentifier(id) => Ok(AnyIdentifier::Temporary(id)),
            other => Err(other),
        }
    }
}

macro_rules! operators {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $text:literal),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name { $($variant),* }

        impl $name {
            /// The operator as written in Luau.
            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $text),* }
            }

            /// The operator written `text`, if any.
            pub fn parse(text: &str) -> Option<Self> {
                match text { $($text => Some($name::$variant),)* _ => None }
            }
        }
    };
}

operators!(
    /// A binary operator.
    BinaryOperator {
        Add = "+", Sub = "-", Mul = "*", Div = "/", FloorDiv = "//", Pow = "^", Mod = "%", Concat = "..",
        Lt = "<", Le = "<=", Gt = ">", Ge = ">=", Eq = "==", Ne = "~=", And = "and", Or = "or",
    }
);

operators!(
    /// A unary operator.
    UnaryOperator { Neg = "-", Not = "not", Len = "#" }
);

operators!(
    /// An assignment operator (Luau's compound assignments included).
    AssignmentOperator {
        Assign = "=", Add = "+=", Sub = "-=", Mul = "*=", Div = "/=", FloorDiv = "//=", Mod = "%=", Pow = "^=",
        Concat = "..=",
    }
);

impl Expr {
    /// TypeScript's `isIndexableExpression`: identifiers, indexing, calls and parenthesized expressions, which can
    /// be called or indexed as they are.
    pub fn is_indexable(&self) -> bool {
        matches!(
            self,
            Expr::Identifier(_)
                | Expr::TemporaryIdentifier(_)
                | Expr::ComputedIndex(_)
                | Expr::PropertyAccess(_)
                | Expr::Call(_)
                | Expr::MethodCall(_)
                | Expr::Parenthesized(_)
        )
    }

    /// `isAnyIdentifier`
    pub fn is_any_identifier(&self) -> bool {
        matches!(self, Expr::Identifier(_) | Expr::TemporaryIdentifier(_))
    }

    /// `isSimple`: identifiers and primitive literals. Cheap to repeat and never needs parentheses.
    pub fn is_simple(&self) -> bool {
        self.is_any_identifier() || self.is_simple_primitive()
    }

    /// `isSimplePrimitive`: `nil`, booleans, numbers and strings.
    pub fn is_simple_primitive(&self) -> bool {
        matches!(self, Expr::Nil | Expr::True | Expr::False | Expr::Number(_) | Expr::String(_) | Expr::LateString(_))
    }

    /// `isTable`: array, set, map and mixed table constructors.
    pub fn is_table(&self) -> bool {
        matches!(self, Expr::Array(_) | Expr::Set(_) | Expr::Map(_) | Expr::MixedTable(_))
    }

    /// `isCall`: a call or a method call.
    pub fn is_call(&self) -> bool {
        matches!(self, Expr::Call(_) | Expr::MethodCall(_))
    }

    /// `isWritableExpression`: what can be assigned to.
    pub fn is_writable(&self) -> bool {
        matches!(
            self,
            Expr::Identifier(_) | Expr::TemporaryIdentifier(_) | Expr::PropertyAccess(_) | Expr::ComputedIndex(_)
        )
    }

    /// `isExpressionWithPrecedence`: binary, unary and if-expressions, which may need parentheses.
    pub fn has_precedence(&self) -> bool {
        matches!(self, Expr::Binary(_) | Expr::Unary(_) | Expr::If(_))
    }

    /// `isNone`
    pub fn is_none(&self) -> bool {
        matches!(self, Expr::None)
    }
}

impl Stmt {
    /// `isFinalStatement`: `break`, `continue` and `return`, after which only comments may follow in a block.
    pub fn is_final(&self) -> bool {
        matches!(self, Stmt::Break | Stmt::Continue | Stmt::Return(_))
    }

    /// `isFunctionLike` for statements: function and method declarations.
    pub fn is_function_like(&self) -> bool {
        matches!(self, Stmt::FunctionDeclaration(_) | Stmt::MethodDeclaration(_))
    }

    /// `isComment`
    pub fn is_comment(&self) -> bool {
        matches!(self, Stmt::Comment(_))
    }

    /// The statement's own block, for the kinds that have one (`hasStatements`; an if statement's `then` block).
    pub fn statements(&self) -> Option<&Vec<Stmt>> {
        match self {
            Stmt::Do(s) => Some(&s.statements),
            Stmt::While(s) => Some(&s.statements),
            Stmt::Repeat(s) => Some(&s.statements),
            Stmt::If(s) => Some(&s.statements),
            Stmt::NumericFor(s) => Some(&s.statements),
            Stmt::For(s) => Some(&s.statements),
            Stmt::FunctionDeclaration(s) => Some(&s.statements),
            Stmt::MethodDeclaration(s) => Some(&s.statements),
            _ => None,
        }
    }

    /// Mutable [`Stmt::statements`].
    pub fn statements_mut(&mut self) -> Option<&mut Vec<Stmt>> {
        match self {
            Stmt::Do(s) => Some(&mut s.statements),
            Stmt::While(s) => Some(&mut s.statements),
            Stmt::Repeat(s) => Some(&mut s.statements),
            Stmt::If(s) => Some(&mut s.statements),
            Stmt::NumericFor(s) => Some(&mut s.statements),
            Stmt::For(s) => Some(&mut s.statements),
            Stmt::FunctionDeclaration(s) => Some(&mut s.statements),
            Stmt::MethodDeclaration(s) => Some(&mut s.statements),
            _ => None,
        }
    }
}
