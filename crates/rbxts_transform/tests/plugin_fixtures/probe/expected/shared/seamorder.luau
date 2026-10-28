-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("include"):WaitForChild("RuntimeLib"))
-- a leading comment of the first statement goes above the plugin imports (Flamework: ts.copyComments)
local _runtime = TS.import(script, game:GetService("ReplicatedStorage"), "shared", "runtime")
local other = _runtime.other
local impl = _runtime.impl
local after = { impl("b", 2), other }
return {
	after = after,
}
