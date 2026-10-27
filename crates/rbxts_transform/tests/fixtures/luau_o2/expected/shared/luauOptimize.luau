-- Compiled with roblox-ts v3.0.0
-- Shapes of stock roblox-ts's output that Luau's `--!optimize 2` (Studio's default for scripts) runs otherwise than
-- JavaScript, through Luau engine bugs (docs/tsrs.md, "Luau bugs"). rbxts-rs emits them as stock does: `expected/` is
-- stock's output.
-- a builtin call (FASTCALL) whose local argument a later argument reassigns through a closure
local function pushRebinds()
	local target = { 1, 2, 3 }
	local original = target
	local function rebind()
		target = { 9 }
		return 99
	end
	table.insert(target, rebind())
	return { #original, #target }
end
local function subRebinds()
	local text = "abc"
	local function rebind()
		text = "xyz"
		return 2
	end
	return string.sub(text, rebind())
end
-- a builtin called directly
local function mathMaxRebinds()
	local a = 1
	local function rebind()
		a = 100
		return 2
	end
	return math.max(a, rebind())
end
local function bitOrRebinds()
	local a = 1
	local rebind = function()
		a = 8
		return 2
	end
	return bit32.bor(a, rebind())
end
local function tonumberRebinds()
	local text = "10"
	local function rebind()
		text = "11"
		return 2
	end
	return tonumber(text, rebind())
end
-- an element access with a name, written as the member
local function mathElementRebinds()
	local a = 1
	local function rebind()
		a = 100
		return 2
	end
	return math.max(a, rebind())
end
-- four arguments are no FASTCALL
local function mathMaxFour()
	local a = 1
	local function rebind()
		a = 100
		return 2
	end
	return math.max(a, 0, 0, rebind())
end
-- no closure assigns the argument
local function mathMaxPlain()
	local a = 1
	a = 3
	return math.max(a, math.min(a, 2))
end
-- nothing reassigns the receiver
local function pushPlain()
	local target = { 1 }
	table.insert(target, 2)
	return #target
end
-- a numeric for from a -0 start: constant, from a constant, and from a runtime value
local function negativeZeroLoop()
	for i = -0, 0 do
		return 1 / i
	end
	return 0
end
local START = -0
local function negativeZeroConstantLoop()
	for i = START, 0 do
		return 1 / i
	end
	return 0
end
local box = {
	start = -0,
}
local function negativeZeroRuntimeRange()
	for i = box.start, 0 do
		return 1 / i
	end
	return 0
end
local function negativeZeroRange()
	for i = -0, 0 do
		return 1 / i
	end
	return 0
end
return {
	pushRebinds = pushRebinds,
	subRebinds = subRebinds,
	mathMaxRebinds = mathMaxRebinds,
	bitOrRebinds = bitOrRebinds,
	tonumberRebinds = tonumberRebinds,
	mathElementRebinds = mathElementRebinds,
	mathMaxFour = mathMaxFour,
	mathMaxPlain = mathMaxPlain,
	pushPlain = pushPlain,
	negativeZeroLoop = negativeZeroLoop,
	negativeZeroConstantLoop = negativeZeroConstantLoop,
	negativeZeroRuntimeRange = negativeZeroRuntimeRange,
	negativeZeroRange = negativeZeroRange,
}
