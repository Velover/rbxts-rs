-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local t = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@rbxts", "t", "lib", "ts").t
local Flamework = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Flamework
-- The dedup heuristic (`guardGenerationDedupLimit`, 3 here): types met three times are declared once.
local Vec2_1 = t.interface({
	x = t.number,
	y = t.number,
})
local points = Flamework.createGuard(t.interface({
	a = Vec2_1,
	b = Vec2_1,
	c = Vec2_1,
}))
local Vec2_2 = t.interface({
	x = t.number,
	y = t.number,
})
local nestedPoints = Flamework.createGuard(t.interface({
	a = t.interface({
		p = Vec2_2,
		q = Vec2_2,
	}),
	b = t.interface({
		p = Vec2_2,
		q = Vec2_2,
	}),
	c = t.interface({
		p = Vec2_2,
		q = Vec2_2,
	}),
}))
local dedup = t.union(t.string, t.number)
local unions = Flamework.createGuard(t.interface({
	a = dedup,
	b = dedup,
	c = dedup,
}))
local twice = Flamework.createGuard(t.interface({
	a = t.interface({
		x = t.number,
		y = t.number,
	}),
	b = t.interface({
		x = t.number,
		y = t.number,
	}),
}))
local function inFunction()
	local Vec2_3 = t.interface({
		x = t.number,
		y = t.number,
	})
	return Flamework.createGuard(t.interface({
		a = Vec2_3,
		b = Vec2_3,
		c = Vec2_3,
	}))
end
return {
	points = points,
	nestedPoints = nestedPoints,
	unions = unions,
	twice = twice,
	inFunction = inFunction,
}
