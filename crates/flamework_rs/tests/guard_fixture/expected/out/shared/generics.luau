-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local t = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@rbxts", "t", "lib", "ts").t
local Flamework = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Flamework
-- Generic types, resolved through the checker; type parameters through their constraints.
local wrapped = Flamework.createGuard(t.interface({
	value = t.number,
	list = t.array(t.number),
}))
local maybe = Flamework.createGuard(t.optional(t.string))
local dedup = t.boolean
local pairOf = Flamework.createGuard(t.strictArray(t.string, t.interface({
	value = dedup,
	list = t.array(dedup),
})))
local v_Vector2 = t.Vector2
local keyed = Flamework.createGuard(t.interface({
	left = v_Vector2,
	right = v_Vector2,
}))
local function constrained()
	return Flamework.createGuard(t.string)
end
local function genericObject()
	return Flamework.createGuard(t.interface({
		item = t.interface({
			id = t.number,
		}),
		items = t.array(t.interface({
			id = t.number,
		})),
	}))
end
return {
	constrained = constrained,
	genericObject = genericObject,
	wrapped = wrapped,
	maybe = maybe,
	pairOf = pairOf,
	keyed = keyed,
}
