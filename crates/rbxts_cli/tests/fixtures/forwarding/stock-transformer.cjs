// A TS transformer mirroring the rbxts-rs probe plugin's diagnostic ops (#15), reported as the probe reports them
// (code " roblox-ts", the slot a plugin diagnostic without a source renders in):
//   probeFail()                -> an error at the call, "probe: fail was called" (probe-fail)
//   probeDiagSame(v, "text")   -> a warning at the call with the text
//   probeBoom()                -> throws (probe-panic)
// With the tsconfig entry option "forward": true it forwards checker diagnostics as Flamework's transformer does
// (transformer.ts: for a file whose getPreEmitDiagnostics has an error, every located one through
// context.addDiagnostic, explained, and the file returned untransformed); the explanation mirrors the probe's
// explain_checker_diagnostic: a message naming probeLoud reports "probe: an error while explaining" at the file, one
// naming probeBroken is replaced by "probe explained: <text>\nDeclare probeBroken first." at the file.
"use strict";
const ts = require("typescript");

module.exports = function (program, config) {
	const forward = !!(config && config.forward);
	return (context) => {
		const at = (node, category, messageText) => ({
			category,
			file: ts.getSourceFileOfNode(node),
			messageText,
			start: node.getStart(),
			length: node.getWidth(),
			code: " roblox-ts",
		});
		return (file) => {
			if (forward) {
				const original = ts.getParseTreeNode(file, ts.isSourceFile);
				if (original) {
					const pre = ts.getPreEmitDiagnostics(program, original);
					if (pre.some((d) => d.category === ts.DiagnosticCategory.Error)) {
						for (const d of pre) {
							if (d.file === undefined || d.start === undefined) continue;
							const text = ts.flattenDiagnosticMessageText(d.messageText, "\n");
							if (text.includes("probeLoud")) {
								context.addDiagnostic(at(original, ts.DiagnosticCategory.Error, "probe: an error while explaining"));
								context.addDiagnostic(d);
							} else if (text.includes("probeBroken")) {
								context.addDiagnostic(at(original, ts.DiagnosticCategory.Error, `probe explained: ${text}\nDeclare probeBroken first.`));
							} else {
								context.addDiagnostic(d);
							}
						}
						return file;
					}
				}
			}
			const visit = (node) => {
				if (ts.isCallExpression(node) && ts.isIdentifier(node.expression)) {
					const callee = node.expression.text;
					if (callee === "probeFail") context.addDiagnostic(at(node, ts.DiagnosticCategory.Error, "probe: fail was called"));
					if (callee === "probeBoom") throw new Error("probe: boom");
					if (callee === "probeDiagSame" && node.arguments.length >= 2 && ts.isStringLiteral(node.arguments[1]))
						context.addDiagnostic(at(node, ts.DiagnosticCategory.Warning, node.arguments[1].text));
				}
				return ts.visitEachChild(node, visit, context);
			};
			return ts.visitEachChild(file, visit, context);
		};
	};
};
