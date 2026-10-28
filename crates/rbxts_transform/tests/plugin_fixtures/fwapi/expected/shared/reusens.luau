-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("include"):WaitForChild("RuntimeLib"))
-- import_named reuses a user import as Flamework's TS transformer does (addFileImport): a namespace import is not
-- reused (the plugin's import is a local of its own beside it)
local _impa = TS.import(script, game:GetService("ReplicatedStorage"), "shared", "impa")
local u_1 = _impa.u
local t = _impa.t
local A = TS.import(script, game:GetService("ReplicatedStorage"), "shared", "impa")
local x = { A.u, u_1, t }
return {
	x = x,
}
