// Luau's `--!optimize 2` (Studio's default for scripts) miscompiles two shapes of stock roblox-ts's output
// (`luau-o2-fastcall`, `luau-o2-negzero`); `runtime/check.luau` compiles every module at that level.

// a builtin call (FASTCALL) reads a local argument's register after the later arguments ran: the receiver is
// captured first when one of them may reassign it
export function pushRebinds() {
	let target = [1, 2, 3];
	const original = target;
	function rebind() {
		target = [9];
		return 99;
	}
	target.push(rebind());
	return [original.size(), target.size()];
}

export function subRebinds() {
	let text = "abc";
	function rebind() {
		text = "xyz";
		return 2;
	}
	return text.sub(rebind());
}

// nothing reassigns the receiver: stock's call
export function pushPlain() {
	const target = [1];
	target.push(2);
	return target.size();
}

// a constant -0 loop start is folded to 0: such a loop is the while loop
export function negativeZeroLoop() {
	for (let i = -0; i <= 0; i++) {
		return 1 / i;
	}
	return 0;
}

const START = -0;
export function negativeZeroConstantLoop() {
	for (let i = START; i <= 0; i++) {
		return 1 / i;
	}
	return 0;
}
