-- Compiled with roblox-ts v3.0.0
-- Luau's `--!optimize 2` (Studio's default for scripts) miscompiles two shapes of stock roblox-ts's output
-- (`luau-o2-fastcall`, `luau-o2-negzero`); `runtime/check.luau` compiles every module at that level.
-- a builtin call (FASTCALL) reads a local argument's register after the later arguments ran: the receiver is
-- captured first when one of them may reassign it
local function pushRebinds()
	local target = { 1, 2, 3 }
	local original = target
	local function rebind()
		target = { 9 }
		return 99
	end
	local _exp = target
	table.insert(_exp, rebind())
	return { #original, #target }
end
local function subRebinds()
	local text = "abc"
	local function rebind()
		text = "xyz"
		return 2
	end
	local _exp = text
	return string.sub(_exp, rebind())
end
-- nothing reassigns the receiver: stock's call
local function pushPlain()
	local target = { 1 }
	table.insert(target, 2)
	return #target
end
-- a constant -0 loop start is folded to 0: such a loop is the while loop
local function negativeZeroLoop()
	do
		local i = -0
		local _shouldIncrement = false
		while true do
			if _shouldIncrement then
				i += 1
			else
				_shouldIncrement = true
			end
			if not (i <= 0) then
				break
			end
			return 1 / i
		end
	end
	return 0
end
local START = -0
local function negativeZeroConstantLoop()
	do
		local i = START
		local _shouldIncrement = false
		while true do
			if _shouldIncrement then
				i += 1
			else
				_shouldIncrement = true
			end
			if not (i <= 0) then
				break
			end
			return 1 / i
		end
	end
	return 0
end
return {
	pushRebinds = pushRebinds,
	subRebinds = subRebinds,
	pushPlain = pushPlain,
	negativeZeroLoop = negativeZeroLoop,
	negativeZeroConstantLoop = negativeZeroConstantLoop,
}
