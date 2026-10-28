-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("include"):WaitForChild("RuntimeLib"))
-- only the file's leading run of import declarations is searched: an import after another statement is not reused
local t_1 = TS.import(script, game:GetService("ReplicatedStorage"), "shared", "impb").t
local u = TS.import(script, game:GetService("ReplicatedStorage"), "shared", "impa").u
local first = u
local t = TS.import(script, game:GetService("ReplicatedStorage"), "shared", "impb").t
local x = { t, t_1, u }
return {
	first = first,
	x = x,
}
