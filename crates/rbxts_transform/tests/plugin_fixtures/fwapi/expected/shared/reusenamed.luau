-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("include"):WaitForChild("RuntimeLib"))
-- import_named reuses a named import of the same specifier, also under another local name
local _impa = TS.import(script, game:GetService("ReplicatedStorage"), "shared", "impa")
local u = _impa.u
local tt = _impa.t
local x = { u, tt, u, tt }
return {
	x = x,
}
