-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local t = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@rbxts", "t", "lib", "ts").t
local Flamework = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Flamework
-- TypeScript enums and Roblox enums.
local Color
do
	local _inverse = {}
	Color = setmetatable({}, {
		__index = _inverse,
	})
	Color.Red = 0
	_inverse[0] = "Red"
	Color.Green = 1
	_inverse[1] = "Green"
	Color.Blue = 2
	_inverse[2] = "Blue"
end
local Mode = {
	Fast = "fast",
	Slow = "slow",
}
local color = Flamework.createGuard(t.literalList({ 0, 1, 2 }))
local mode = Flamework.createGuard(t.literal("fast", "slow"))
local direction = Flamework.createGuard(t.literal(1, 2))
local red = Flamework.createGuard(t.literal(0))
local someColors = Flamework.createGuard(t.literal(0, 2))
local optionalMode = Flamework.createGuard(t.optional(t.literal("fast", "slow")))
local material = Flamework.createGuard(t.enum(Enum.Material))
local plastic = Flamework.createGuard(t.literal(Enum.Material.Plastic))
local plasticOrWood = Flamework.createGuard(t.literal(Enum.Material.Plastic, Enum.Material.Wood))
local plasticOrNumber = Flamework.createGuard(t.union(t.number, t.literal(Enum.Material.Plastic, Enum.Material.Wood)))
local sortOrder = Flamework.createGuard(t.enum(Enum.SortOrder))
local keyCode = Flamework.createGuard(t.enum(Enum.KeyCode))
local optionalMaterial = Flamework.createGuard(t.optional(t.enum(Enum.Material)))
return {
	color = color,
	mode = mode,
	direction = direction,
	red = red,
	someColors = someColors,
	optionalMode = optionalMode,
	material = material,
	plastic = plastic,
	plasticOrWood = plasticOrWood,
	plasticOrNumber = plasticOrNumber,
	sortOrder = sortOrder,
	keyCode = keyCode,
	optionalMaterial = optionalMaterial,
}
