-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local _markers = TS.import(script, game:GetService("ReplicatedStorage"), "TS", "markers")
local callsite = _markers.callsite
local where = _markers.where
-- Positions are UTF-16: "é" is one unit, "😀" two, as TypeScript counts them.
local greeting = "héllo 😀"
local here = where(4, 57, 7, "f0702dac-c6be-52c3-988f-4b7749644ed9", "where()")
local names = {
	["日本"] = callsite("9b279f6a-6f3e-5b21-9578-685daa639a08"),
	["😀"] = where(6, 48, 49, "442995bd-bade-5b75-9299-c2748ba04fd0", "where(undefined, undefined, undefined, undefined)"),
}
local function nested()
	local value = "😀😀"
	local at = where(9, 49, 7, "8e30b7d2-0b29-595f-93b5-9e89216edc07", "where()")
	return { value, callsite("60370e7e-27db-5e36-865a-2697cf860efd"), at }
end
return {
	nested = nested,
	greeting = greeting,
	here = here,
	names = names,
}
