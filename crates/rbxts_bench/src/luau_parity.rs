//! `rbxts-bench luau-parity`: renders Luau trees dumped from roblox-ts (`tools/dump-luau-ast.cjs`) with
//! `rbxts_luau` and compares the text with what roblox-ts's `renderAST` produced, byte for byte.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

use rbxts_luau::*;
use serde_json::Value;

pub const USAGE: &str = "usage: rbxts-bench luau-parity <dump.jsonl>... [--show N] [--repeat N] [--threads N]";

struct Args {
    dumps: Vec<String>,
    show: usize,
    repeat: usize,
    /// Render on this many threads (files dealt round-robin), as the compiler will after plugin finish.
    threads: usize,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut out = Args { dumps: Vec::new(), show: 10, repeat: 5, threads: 1 };
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .ok_or_else(|| format!("{name} needs a value"))?
                .parse::<usize>()
                .map_err(|e| format!("{name}: {e}"))
        };
        match arg.as_str() {
            "--show" => out.show = value("--show")?,
            "--repeat" => out.repeat = value("--repeat")?.max(1),
            "--threads" => out.threads = value("--threads")?.max(1),
            _ if arg.starts_with('-') => return Err(format!("unknown argument {arg}\n{USAGE}")),
            _ => out.dumps.push(arg),
        }
    }
    if out.dumps.is_empty() {
        return Err(USAGE.into());
    }
    Ok(out)
}

/// One dumped file: its name, roblox-ts's text and the tree.
struct Case {
    file: String,
    expected: String,
    ast: Vec<Stmt>,
}

pub fn run(args: impl Iterator<Item = String>) -> i32 {
    let args = match parse_args(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("rbxts-bench luau-parity: {e}");
            return 2;
        }
    };
    let mut failed = false;
    for dump in &args.dumps {
        failed |= !compare_dump(dump, &args);
    }
    i32::from(failed)
}

fn compare_dump(dump: &str, args: &Args) -> bool {
    let text = match std::fs::read_to_string(dump) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{dump}: {e}");
            return false;
        }
    };
    let start = Instant::now();
    let mut cases = Vec::new();
    for (i, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
        match load_case(line) {
            Ok(case) => cases.push(case),
            Err(e) => {
                eprintln!("{dump}:{}: {e}", i + 1);
                return false;
            }
        }
    }
    let load_time = start.elapsed();

    // Render every file `repeat` times and keep the fastest pass; the first pass's output is compared.
    let late_strings = LateStrings::new();
    let mut outputs = Vec::new();
    let mut best = Duration::MAX;
    for pass in 0..args.repeat {
        let start = Instant::now();
        let rendered = render_all(&cases, &late_strings, args.threads);
        best = best.min(start.elapsed());
        if pass == 0 {
            outputs = rendered;
        }
    }

    let mut identical = 0;
    let mut bytes = 0;
    let mut shown = 0;
    for (case, output) in cases.iter().zip(&outputs) {
        bytes += case.expected.len();
        let problem = match output {
            Ok(actual) if *actual == case.expected => {
                identical += 1;
                continue;
            }
            Ok(actual) => first_difference(&case.expected, actual),
            Err(panic) => format!("    render panicked: {panic}"),
        };
        if shown < args.show {
            println!("  DIFF {}\n{problem}", case.file);
            shown += 1;
        }
    }
    println!(
        "{dump}: {} files, {identical} identical, {} different; {:.1} MiB of Luau; render {:.2} ms (best of {}, {} thread{}), load {:.0} ms",
        cases.len(),
        cases.len() - identical,
        bytes as f64 / (1024.0 * 1024.0),
        best.as_secs_f64() * 1e3,
        args.repeat,
        args.threads,
        if args.threads == 1 { "" } else { "s" },
        load_time.as_secs_f64() * 1e3,
    );
    identical == cases.len()
}

/// Renders every case, catching panics (TypeScript's asserts) per file.
fn render_all(cases: &[Case], late_strings: &LateStrings, threads: usize) -> Vec<Result<String, String>> {
    let render_one = |case: &Case| {
        catch_unwind(AssertUnwindSafe(|| render(&case.ast, late_strings))).map_err(|e| {
            e.downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "panic".into())
        })
    };
    if threads == 1 {
        return cases.iter().map(render_one).collect();
    }
    let mut outputs: Vec<Option<Result<String, String>>> = (0..cases.len()).map(|_| None).collect();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|t| {
                std::thread::Builder::new()
                    .stack_size(rbxts_project::STACK_SIZE)
                    .spawn_scoped(scope, move || {
                        cases
                            .iter()
                            .enumerate()
                            .skip(t)
                            .step_by(threads)
                            .map(|(i, c)| (i, render_one(c)))
                            .collect::<Vec<_>>()
                    })
                    .expect("spawn a render thread")
            })
            .collect();
        for worker in workers {
            for (i, output) in worker.join().expect("render threads catch their panics") {
                outputs[i] = Some(output);
            }
        }
    });
    outputs.into_iter().map(|o| o.expect("every case is rendered")).collect()
}

fn first_difference(expected: &str, actual: &str) -> String {
    let clip = |s: &str| s.chars().take(160).collect::<String>();
    let (mut e, mut a) = (expected.split('\n'), actual.split('\n'));
    for line in 1.. {
        match (e.next(), a.next()) {
            (Some(x), Some(y)) if x == y => {}
            (x, y) => {
                return format!(
                    "    line {line}\n    expected: {:?}\n    actual:   {:?}",
                    x.map(clip).unwrap_or_else(|| "<end>".into()),
                    y.map(clip).unwrap_or_else(|| "<end>".into())
                );
            }
        }
    }
    unreachable!()
}

fn load_case(line: &str) -> Result<Case, String> {
    // Long `..` chains nest deeper than serde_json's default limit of 128.
    let mut de = serde_json::Deserializer::from_str(line);
    de.disable_recursion_limit();
    let value: Value = serde::Deserialize::deserialize(&mut de).map_err(|e| e.to_string())?;
    let file = value["file"].as_str().unwrap_or("?").to_owned();
    let expected = value["text"].as_str().ok_or("no `text`")?.to_owned();
    let ast = statements(&value["ast"]).map_err(|e| format!("{file}: {e}"))?;
    Ok(Case { file, expected, ast })
}

type R<T> = Result<T, String>;

fn kind(v: &Value) -> R<&str> {
    v.get("k").and_then(Value::as_str).ok_or_else(|| format!("not a node: {}", clip_json(v)))
}

fn clip_json(v: &Value) -> String {
    v.to_string().chars().take(120).collect()
}

fn field<'v>(v: &'v Value, name: &str) -> R<&'v Value> {
    v.get(name).ok_or_else(|| format!("{} has no `{name}`", kind(v).unwrap_or("?")))
}

fn str_field(v: &Value, name: &str) -> R<String> {
    field(v, name)?.as_str().map(str::to_owned).ok_or_else(|| format!("`{name}` is not a string"))
}

fn bool_field(v: &Value, name: &str) -> R<bool> {
    field(v, name)?.as_bool().ok_or_else(|| format!("`{name}` is not a boolean"))
}

/// A field that holds a node or a list of nodes (TypeScript's `X | List<X>`).
fn one_or_many(v: &Value) -> Vec<&Value> {
    match v {
        Value::Array(items) => items.iter().collect(),
        Value::Null => Vec::new(),
        other => vec![other],
    }
}

fn array<'v>(v: &'v Value, name: &str) -> R<&'v Vec<Value>> {
    field(v, name)?.as_array().ok_or_else(|| format!("`{name}` is not a list"))
}

fn statements(v: &Value) -> R<Vec<Stmt>> {
    v.as_array().ok_or("a statement list is not a list")?.iter().map(statement).collect()
}

fn exprs(values: impl IntoIterator<Item = impl std::ops::Deref<Target = Value>>) -> R<Vec<Expr>> {
    values.into_iter().map(|v| expr(&v)).collect()
}

fn any_identifier(v: &Value) -> R<AnyIdentifier> {
    AnyIdentifier::try_from(expr(v)?).map_err(|e| format!("expected an identifier, got {e:?}"))
}

fn any_identifiers(values: Vec<&Value>) -> R<Vec<AnyIdentifier>> {
    values.into_iter().map(any_identifier).collect()
}

fn statement(v: &Value) -> R<Stmt> {
    let block = |name: &str| statements(field(v, name)?);
    Ok(match kind(v)? {
        "Assignment" => {
            let operator = str_field(v, "operator")?;
            Stmt::Assignment(Box::new(Assignment {
                left: exprs(one_or_many(field(v, "left")?))?,
                operator: AssignmentOperator::parse(&operator).ok_or_else(|| format!("operator {operator}"))?,
                right: exprs(one_or_many(field(v, "right")?))?,
            }))
        }
        "BreakStatement" => Stmt::Break,
        "ContinueStatement" => Stmt::Continue,
        "CallStatement" => Stmt::Call(expr(field(v, "expression")?)?),
        "DoStatement" => Stmt::Do(DoStatement { statements: block("statements")? }),
        "WhileStatement" => Stmt::While(Box::new(WhileStatement {
            condition: expr(field(v, "condition")?)?,
            statements: block("statements")?,
        })),
        "RepeatStatement" => Stmt::Repeat(Box::new(RepeatStatement {
            condition: expr(field(v, "condition")?)?,
            statements: block("statements")?,
        })),
        "IfStatement" => Stmt::If(Box::new(if_statement(v)?)),
        "NumericForStatement" => Stmt::NumericFor(Box::new(NumericForStatement {
            id: any_identifier(field(v, "id")?)?,
            start: expr(field(v, "start")?)?,
            end: expr(field(v, "end")?)?,
            step: v.get("step").map(expr).transpose()?,
            statements: block("statements")?,
        })),
        "ForStatement" => Stmt::For(Box::new(ForStatement {
            ids: any_identifiers(array(v, "ids")?.iter().collect())?,
            expression: expr(field(v, "expression")?)?,
            statements: block("statements")?,
        })),
        "FunctionDeclaration" => Stmt::FunctionDeclaration(Box::new(FunctionDeclaration {
            localize: bool_field(v, "localize")?,
            name: expr(field(v, "name")?)?,
            parameters: any_identifiers(array(v, "parameters")?.iter().collect())?,
            has_dot_dot_dot: bool_field(v, "hasDotDotDot")?,
            statements: block("statements")?,
        })),
        "MethodDeclaration" => Stmt::MethodDeclaration(Box::new(MethodDeclaration {
            expression: expr(field(v, "expression")?)?,
            name: str_field(v, "name")?,
            parameters: any_identifiers(array(v, "parameters")?.iter().collect())?,
            has_dot_dot_dot: bool_field(v, "hasDotDotDot")?,
            statements: block("statements")?,
        })),
        "VariableDeclaration" => Stmt::VariableDeclaration(Box::new(VariableDeclaration {
            left: any_identifiers(one_or_many(field(v, "left")?))?,
            right: match v.get("right") {
                Some(right) => exprs(one_or_many(right))?,
                None => Vec::new(),
            },
        })),
        "ReturnStatement" => {
            Stmt::Return(ReturnStatement { expressions: exprs(one_or_many(field(v, "expression")?))? })
        }
        "Comment" => Stmt::Comment(str_field(v, "text")?),
        other => return Err(format!("not a statement kind: {other}")),
    })
}

fn if_statement(v: &Value) -> R<IfStatement> {
    let else_body = field(v, "elseBody")?;
    Ok(IfStatement {
        condition: expr(field(v, "condition")?)?,
        statements: statements(field(v, "statements")?)?,
        else_body: match else_body {
            Value::Array(_) => ElseBody::Block(statements(else_body)?),
            _ => ElseBody::If(Box::new(if_statement(else_body)?)),
        },
    })
}

fn map_field(v: &Value) -> R<MapField> {
    if kind(v)? != "MapField" {
        return Err(format!("expected a MapField, got {}", kind(v)?));
    }
    Ok(MapField { index: expr(field(v, "index")?)?, value: expr(field(v, "value")?)? })
}

fn expr(v: &Value) -> R<Expr> {
    let sub = |name: &str| expr(field(v, name)?);
    let list = |name: &str| exprs(array(v, name)?.iter());
    Ok(match kind(v)? {
        "Identifier" => Expr::Identifier(Identifier { name: str_field(v, "name")?, facts: Facts::default() }),
        "TemporaryIdentifier" => Expr::TemporaryIdentifier(TemporaryIdentifier {
            name: str_field(v, "name")?,
            id: field(v, "id")?.as_u64().ok_or("temporary id is not a number")? as u32,
            facts: Facts::default(),
        }),
        "ComputedIndexExpression" => Expr::ComputedIndex(Box::new(ComputedIndexExpression {
            expression: sub("expression")?,
            index: sub("index")?,
            facts: Facts::default(),
        })),
        "PropertyAccessExpression" => Expr::PropertyAccess(Box::new(PropertyAccessExpression {
            expression: sub("expression")?,
            name: str_field(v, "name")?,
            facts: Facts::default(),
        })),
        "CallExpression" => {
            Expr::Call(Box::new(CallExpression { expression: sub("expression")?, args: list("args")?, facts: Facts::default() }))
        }
        "MethodCallExpression" => Expr::MethodCall(Box::new(MethodCallExpression {
            name: str_field(v, "name")?,
            expression: sub("expression")?,
            args: list("args")?,
            facts: Facts::default(),
        })),
        "ParenthesizedExpression" => Expr::Parenthesized(Box::new(ParenthesizedExpression { expression: sub("expression")?, facts: Facts::default() })),
        "None" => Expr::None,
        "NilLiteral" => Expr::Nil,
        "FalseLiteral" => Expr::False,
        "TrueLiteral" => Expr::True,
        "NumberLiteral" => Expr::Number(NumberLiteral { value: str_field(v, "value")? }),
        "StringLiteral" => Expr::String(StringLiteral {
            value: str_field(v, "value")?,
            quote: match v.get("quote").and_then(Value::as_str) {
                None => None,
                Some("\"") => Some(Quote::Double),
                Some("'") => Some(Quote::Single),
                Some(q) => return Err(format!("quote {q}")),
            },
        }),
        "VarArgsLiteral" => Expr::VarArgs,
        "FunctionExpression" => Expr::Function(Box::new(FunctionExpression {
            parameters: any_identifiers(array(v, "parameters")?.iter().collect())?,
            has_dot_dot_dot: bool_field(v, "hasDotDotDot")?,
            statements: statements(field(v, "statements")?)?,
            facts: Facts::default(),
        })),
        "BinaryExpression" => {
            let operator = str_field(v, "operator")?;
            Expr::Binary(Box::new(BinaryExpression {
                left: sub("left")?,
                operator: BinaryOperator::parse(&operator).ok_or_else(|| format!("operator {operator}"))?,
                right: sub("right")?,
                facts: Facts::default(),
            }))
        }
        "UnaryExpression" => {
            let operator = str_field(v, "operator")?;
            Expr::Unary(Box::new(UnaryExpression {
                operator: UnaryOperator::parse(&operator).ok_or_else(|| format!("operator {operator}"))?,
                expression: sub("expression")?,
                facts: Facts::default(),
            }))
        }
        "IfExpression" => Expr::If(Box::new(IfExpression {
            condition: sub("condition")?,
            expression: sub("expression")?,
            alternative: sub("alternative")?,
            facts: Facts::default(),
        })),
        "InterpolatedString" => Expr::InterpolatedString(InterpolatedString {
            parts: array(v, "parts")?
                .iter()
                .map(|p| match kind(p)? {
                    "InterpolatedStringPart" => Ok(InterpolatedStringPart::Text(str_field(p, "text")?)),
                    _ => Ok(InterpolatedStringPart::Expression(expr(p)?)),
                })
                .collect::<R<_>>()?,
            facts: Facts::default(),
        }),
        "Array" => Expr::Array(list("members")?),
        "Set" => Expr::Set(list("members")?),
        "Map" => Expr::Map(array(v, "fields")?.iter().map(map_field).collect::<R<_>>()?),
        "MixedTable" => Expr::MixedTable(
            array(v, "fields")?
                .iter()
                .map(|f| match kind(f)? {
                    "MapField" => Ok(TableField::Field(map_field(f)?)),
                    _ => Ok(TableField::Value(expr(f)?)),
                })
                .collect::<R<_>>()?,
        ),
        other => return Err(format!("not an expression kind: {other}")),
    })
}

#[cfg(test)]
mod tests {
    /// The hand-written edge cases (`tools/luau-edge-cases.cjs`), as roblox-ts's renderer renders them.
    #[test]
    fn edge_cases_render_as_roblox_ts_does() {
        let fixture = include_str!("../fixtures/luau-edge-cases.jsonl");
        let mut count = 0;
        for line in fixture.lines().filter(|l| !l.trim().is_empty()) {
            let case = super::load_case(line).expect("the fixture loads");
            let actual = rbxts_luau::render_ast(&case.ast);
            assert!(actual == case.expected, "{}:\n{}", case.file, super::first_difference(&case.expected, &actual));
            count += 1;
        }
        assert!(count > 10);
    }
}
