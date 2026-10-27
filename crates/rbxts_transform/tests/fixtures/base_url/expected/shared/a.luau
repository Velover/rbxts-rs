-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("include"):WaitForChild("RuntimeLib"))
-- `baseUrl` (tests/fixtures.rs, `base_url`): a name only `baseUrl` resolves, a `paths` pattern whose substitution is
-- relative to `baseUrl`, and a declaration that names a type of a module it does not import.
local m = TS.import(script, game:GetService("ReplicatedStorage"), "shared", "mod").m
local _thing = TS.import(script, game:GetService("ReplicatedStorage"), "lib", "thing")
local t = _thing.t
local getThing = _thing.getThing
local a = m + t
local thing = getThing()
return {
	a = a,
	thing = thing,
}
