-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local t = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@rbxts", "t", "lib", "ts").t
local Flamework = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Flamework
-- Template literal and string mapping types.
local suffix = Flamework.createGuard(t.match("^.*%-id$"))
local prefixNumber = Flamework.createGuard(t.match("^item_.*$"))
local special = Flamework.createGuard(t.match("^a%.b%(.*%)%%%[x%]%*%+%?%^%$$"))
local unionTemplate = Flamework.createGuard(t.union(t.match("^a%-.*$"), t.match("^b%-.*$")))
local lower = Flamework.createGuard(t.string)
local templateOrNumber = Flamework.createGuard(t.union(t.number, t.match("^#.*$")))
return {
	suffix = suffix,
	prefixNumber = prefixNumber,
	special = special,
	unionTemplate = unionTemplate,
	lower = lower,
	templateOrNumber = templateOrNumber,
}
