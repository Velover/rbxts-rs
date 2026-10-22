-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local t = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@rbxts", "t", "lib", "ts").t
local Flamework = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Flamework
-- Instance types, with children from intersections and interfaces.
local part = Flamework.createGuard(t.instanceIsA("Part"))
local instance = Flamework.createGuard(t.instanceIsA("Instance"))
local basePart = Flamework.createGuard(t.instanceIsA("BasePart"))
local modelWithHead = Flamework.createGuard(t.intersection(t.instanceIsA("Model"), t.children({
	Head = t.instanceIsA("Part"),
	Humanoid = t.instanceIsA("Humanoid"),
})))
local character = Flamework.createGuard(t.intersection(t.instanceIsA("Model"), t.children({
	Head = t.instanceIsA("Part"),
	HumanoidRootPart = t.instanceIsA("BasePart"),
	Animate = t.intersection(t.instanceIsA("LocalScript"), t.children({
		idle = t.instanceIsA("Animation"),
	})),
})))
local partOrModel = Flamework.createGuard(t.union(t.instanceIsA("Model"), t.instanceIsA("Part")))
local optionalPart = Flamework.createGuard(t.optional(t.instanceIsA("Part")))
local parts = Flamework.createGuard(t.array(t.instanceIsA("Part")))
return {
	part = part,
	instance = instance,
	basePart = basePart,
	modelWithHead = modelWithHead,
	character = character,
	partOrModel = partOrModel,
	optionalPart = optionalPart,
	parts = parts,
}
