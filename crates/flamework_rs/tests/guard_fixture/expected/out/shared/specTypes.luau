-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local t = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@rbxts", "t", "lib", "ts").t
local Flamework = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Flamework
-- The types of the specs' type checks (packages/specs/src/typeChecks/codecs.ts), as guards.
local u8 = Flamework.createGuard(t.number)
local varint = Flamework.createGuard(t.number)
local string8 = Flamework.createGuard(t.string)
local bytes = Flamework.createGuard(t.typeof("buffer"))
local literals = Flamework.createGuard(t.literalList({ "a", "b", "c" }))
local circle = Flamework.createGuard(t.interface({
	kind = t.literal("circle"),
	r = t.number,
}))
local frame = Flamework.createGuard(t.CFrame)
local list = Flamework.createGuard(t.array(t.number))
local set = Flamework.createGuard(t.set(t.string))
local map = Flamework.createGuard(t.map(t.string, t.number))
local maybe = Flamework.createGuard(t.optional(t.number))
local either = Flamework.createGuard(t.union(t.string, t.number))
local pair = Flamework.createGuard(t.strictArray(t.number, t.string))
local points = Flamework.createGuard(t.array(t.interface({
	x = t.number,
	y = t.number,
})))
local entity = Flamework.createGuard(t.interface({
	id = t.number,
	name = t.string,
	tags = t.array(t.string),
	pos = t.interface({
		x = t.number,
		y = t.number,
	}),
	spot = t.optional(t.Vector3),
	flag = t.boolean,
	owner = t.interface({
		label = t.string,
	}),
}))
local shape = Flamework.createGuard(t.union(t.interface({
	kind = t.literal("circle"),
	r = t.number,
}), t.interface({
	kind = t.literal("rect"),
	w = t.number,
	h = t.number,
	label = t.string,
})))
local plastic = Flamework.createGuard(t.literal(Enum.Material.Plastic))
local plasticOrWood = Flamework.createGuard(t.literal(Enum.Material.Plastic, Enum.Material.Wood))
local plasticOrNumber = Flamework.createGuard(t.union(t.number, t.literal(Enum.Material.Plastic, Enum.Material.Wood)))
local v_Vector3 = t.Vector3
local dedup = t.boolean
local moveArgs = Flamework.createGuard(t.strictArray(t.number, t.interface({
	id = t.number,
	name = t.string,
	tags = t.array(t.string),
	pos = t.interface({
		x = t.number,
		y = t.number,
	}),
	spot = t.optional(v_Vector3),
	flag = dedup,
	owner = t.interface({
		label = t.string,
	}),
}), dedup))
local manyArgs = Flamework.createGuard((function(element, rest)
	return function(value)
		if not (type(value) == "table") then
			return false
		end
		local size = 0
		for key in pairs(value) do
			if not (type(key) == "number") or key < 1 or key % 1 ~= 0 then
				return false
			end
			if key > size then
				size = key
			end
		end
		if size < 1 then
			return false
		end
		local list_1 = value
		if not element(list_1[1]) then
			return false
		end
		for index = 2, size do
			if not rest(list_1[index]) then
				return false
			end
		end
		return true
	end
end)(t.string, t.number))
return {
	u8 = u8,
	varint = varint,
	string8 = string8,
	bytes = bytes,
	literals = literals,
	circle = circle,
	frame = frame,
	list = list,
	set = set,
	map = map,
	maybe = maybe,
	either = either,
	pair = pair,
	points = points,
	entity = entity,
	shape = shape,
	plastic = plastic,
	plasticOrWood = plasticOrWood,
	plasticOrNumber = plasticOrNumber,
	moveArgs = moveArgs,
	manyArgs = manyArgs,
}
