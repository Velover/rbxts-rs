-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local t = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@rbxts", "t", "lib", "ts").t
local Flamework = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Flamework
-- Interfaces, type literals and their keys.
local point = Flamework.createGuard(t.interface({
	x = t.number,
	y = t.number,
}))
local literalObject = Flamework.createGuard(t.interface({
	a = t.number,
	b = t.optional(t.string),
	c = t.interface({
		x = t.number,
		y = t.number,
	}),
}))
local nestedObject = Flamework.createGuard(t.interface({
	inner = t.interface({
		deep = t.interface({
			value = t.number,
		}),
	}),
}))
local numericKeys = Flamework.createGuard(t.interface({
	[10] = t.string,
	[1.5] = t.number,
	["two words"] = t.boolean,
	[-1] = t.interface({
		x = t.number,
		y = t.number,
	}),
}))
local ignoredFields = Flamework.createGuard(t.interface({
	a = t.number,
}))
local emptyObject = Flamework.createGuard(t.any)
local withMethod = Flamework.createGuard(t.interface({
	run = t.callback,
	value = t.number,
}))
local intersection = Flamework.createGuard(t.intersection(t.interface({
	x = t.number,
	y = t.number,
}), t.interface({
	name = t.string,
	tags = t.optional(t.array(t.string)),
})))
local unionOfObjects = Flamework.createGuard(t.union(t.interface({
	kind = t.literal("a"),
	a = t.number,
}), t.interface({
	kind = t.literal("b"),
	b = t.string,
})))
local picked = Flamework.createGuard(t.interface({
	x = t.number,
}))
local extended = Flamework.createGuard(t.interface({
	label = t.string,
	point = t.interface({
		x = t.number,
		y = t.number,
	}),
	name = t.string,
	tags = t.optional(t.array(t.string)),
}))
local readonlyObject = Flamework.createGuard(t.interface({
	x = t.number,
	y = t.number,
}))
local optionalObject = Flamework.createGuard(t.optional(t.interface({
	x = t.number,
	y = t.number,
})))
-- Inherited properties: TS5 lists the interface's own properties, then the inherited ones it does not redeclare.
local potions = Flamework.createGuard(t.map(t.string, t.interface({
	Type = t.string,
	Duration = t.string,
	Boosts = t.optional(t.interface({
		Xp = t.optional(t.number),
	})),
	DisplayName = t.string,
	Description = t.string,
	Icon = t.string,
	Rarity = t.string,
})))
local buyRequirement = Flamework.createGuard(t.interface({
	Amount = t.number,
	ShovelName = t.string,
	Type = t.string,
}))
local localPotion = Flamework.createGuard(t.interface({
	Type = t.string,
	Duration = t.string,
	DisplayName = t.string,
	Description = t.string,
	Icon = t.string,
	Rarity = t.string,
}))
-- Inherited properties read before the guard is built: the order is still the declared one.
local function describe(card)
	return card.Rarity .. card.Icon
end
local potionCard = Flamework.createGuard(t.interface({
	Kind = t.string,
	Length = t.string,
	Title = t.string,
	Body = t.string,
	Icon = t.string,
	Rarity = t.string,
}))
return {
	describe = describe,
	point = point,
	literalObject = literalObject,
	nestedObject = nestedObject,
	numericKeys = numericKeys,
	ignoredFields = ignoredFields,
	emptyObject = emptyObject,
	withMethod = withMethod,
	intersection = intersection,
	unionOfObjects = unionOfObjects,
	picked = picked,
	extended = extended,
	readonlyObject = readonlyObject,
	optionalObject = optionalObject,
	potions = potions,
	buyRequirement = buyRequirement,
	localPotion = localPotion,
	potionCard = potionCard,
}
