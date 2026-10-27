-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("include"):WaitForChild("RuntimeLib"))
-- A package imported by relative path, linked into the type root several times (tests/fixtures.rs,
-- `junction_automatic_types`): the tsconfig has no `types`, so every type root package is an automatic type directive,
-- and TypeScript 5 meets their links in the type root's directory order: the import goes through the first, `alpha`.
local greet = TS.import(script, game:GetService("ReplicatedStorage"), "include", "node_modules", "@rbxts", "alpha", "index").greet
local x = greet("t")
return {
	x = x,
}
