-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local Modding = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Modding
local _markers = TS.import(script, game:GetService("ReplicatedStorage"), "TS", "markers")
local constant = _markers.constant
local emit = _markers.emit
local keysOf = _markers.keysOf
local labelsOf = _markers.labelsOf
local obfuscated = _markers.obfuscated
local shuffled = _markers.shuffled
-- `Modding.Emit` / `Many`: tuples, arrays of unions, objects (number keys included), literals.
local tuple = emit({ "a", 1, true, false, nil })
local single = emit({ "only" })
local empty = emit({})
local object = emit({
	name = "x",
	[10] = "ten",
	nested = {
		flag = true,
	},
	["two words"] = 2,
})
local record = emit({
	[1] = "n",
	[2] = "n",
})
local inspected = Modding.inspect({
	kind = "inspected",
	list = { "a", "b" },
})
local function sample(first, second, ...)
	local rest = { ... }
	return { first, second, rest }
end
-- Labels: tuple element names, and parameter names.
local labels = labelsOf({ "name", "age" })
local parameterNames = labelsOf({ "first", "second", "rest" })
local unlabelled = labelsOf({ "", "" })
-- Flamework user macro metadata (line 22)
local _constant_22 = {
	shared = true,
}
-- Flamework hoisted this macro's metadata (constant_22) to the file root.
-- Constant: hoisted to the file root inside a function, inline at the root.
local atRoot = constant(_constant_22)
-- Flamework user macro metadata (line 25)
local _constant_25 = {
	shared = true,
	deep = {
		list = { 1, 2 },
	},
}
-- Flamework user macro metadata (line 25)
local _constant_25_1 = {
	again = 1,
}
local function invoke()
	-- Flamework hoisted this macro's metadata (constant_25) to the file root.
	-- Flamework hoisted this macro's metadata (constant_25) to the file root.
	return { constant(_constant_25), constant("plain"), constant(_constant_25_1) }
end
-- Obfuscation intrinsics: with obfuscation off, the keys and order stay.
local keys = keysOf({
	playerJoined = 1,
	playerLeft = 2,
	[3] = "number key",
})
local order = shuffled({ "a", "b", "c" })
local name = obfuscated("playerJoined")
return {
	invoke = invoke,
	tuple = tuple,
	single = single,
	empty = empty,
	object = object,
	record = record,
	inspected = inspected,
	labels = labels,
	parameterNames = parameterNames,
	unlabelled = unlabelled,
	atRoot = atRoot,
	keys = keys,
	order = order,
	name = name,
}
