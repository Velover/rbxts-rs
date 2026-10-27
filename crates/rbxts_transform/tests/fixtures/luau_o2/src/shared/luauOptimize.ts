// Shapes of stock roblox-ts's output that Luau's `--!optimize 2` (Studio's default for scripts) runs otherwise than
// JavaScript (docs/tsrs.md, "Luau behaviour that roblox-ts output can meet"). rbxts-rs emits them as stock does:
// `expected/` is stock's output.

// a builtin call (FASTCALL) whose local argument a later argument reassigns through a closure
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

// a builtin called directly
export function mathMaxRebinds() {
	let a = 1;
	function rebind() {
		a = 100;
		return 2;
	}
	return math.max(a, rebind());
}

export function bitOrRebinds() {
	let a = 1;
	const rebind = () => {
		a = 8;
		return 2;
	};
	return bit32.bor(a, rebind());
}

export function tonumberRebinds() {
	let text = "10";
	function rebind() {
		text = "11";
		return 2;
	}
	return tonumber(text, rebind());
}

// an element access with a name, written as the member
export function mathElementRebinds() {
	let a = 1;
	function rebind() {
		a = 100;
		return 2;
	}
	return math["max"](a, rebind());
}

// four arguments are no FASTCALL
export function mathMaxFour() {
	let a = 1;
	function rebind() {
		a = 100;
		return 2;
	}
	return math.max(a, 0, 0, rebind());
}

// no closure assigns the argument
export function mathMaxPlain() {
	let a = 1;
	a = 3;
	return math.max(a, math.min(a, 2));
}

// nothing reassigns the receiver
export function pushPlain() {
	const target = [1];
	target.push(2);
	return target.size();
}

// a numeric for from a -0 start: constant, from a constant, and from a runtime value
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

const box = { start: -0 };
export function negativeZeroRuntimeRange() {
	for (const i of $range(box.start, 0)) {
		return 1 / i;
	}
	return 0;
}

export function negativeZeroRange() {
	for (const i of $range(-0, 0)) {
		return 1 / i;
	}
	return 0;
}
