-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("include"):WaitForChild("RuntimeLib"))
-- header one
-- header two
-- group two
-- attached
-- doc
local t = TS.import(script, game:GetService("ReplicatedStorage"), "shared", "impb").t
local function h()
	return t
end
return {
	h = h,
}
