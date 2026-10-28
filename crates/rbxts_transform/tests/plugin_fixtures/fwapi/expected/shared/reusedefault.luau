-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("include"):WaitForChild("RuntimeLib"))
-- a named import beside a default one is reused; the default import itself is not (`{ default as … }` is)
local _withdefault = TS.import(script, game:GetService("ReplicatedStorage"), "shared", "withdefault")
local D = _withdefault.default
local named = _withdefault.named
local Dflt = TS.import(script, game:GetService("ReplicatedStorage"), "shared", "withdefault").default
local x = { D, Dflt, named, named, Dflt }
return {
	x = x,
}
