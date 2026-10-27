-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("include"):WaitForChild("RuntimeLib"))
local y = TS.import(script, game:GetService("ReplicatedStorage"), "b", "y").y
local x = y + 1
return {
	x = x,
}
