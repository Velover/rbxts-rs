-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local _markers = TS.import(script, game:GetService("ReplicatedStorage"), "TS", "markers")
local callsite = _markers.callsite
local where = _markers.where
-- Caller metadata: line, character, width, uuid and the source text, in UTF-16.
local atRoot = where(4, 23, 7, "05250e1d-1442-5e3c-961d-a7d91e07d8fb", "where()")
local function inFunction()
	return where(7, 9, 9, "f94b5e38-e5c2-54e3-88ca-50cdeaf50f1c", "where(  )")
end
local Holder
do
	Holder = setmetatable({}, {
		__tostring = function()
			return "Holder"
		end,
	})
	Holder.__index = Holder
	function Holder.new(...)
		local self = setmetatable({}, Holder)
		return self:constructor(...) or self
	end
	function Holder:constructor()
		self.field = callsite("b82178a7-dc81-51f3-9b0e-49797e71ea83")
	end
	function Holder:method()
		return { callsite("d1e9ec1d-41cf-52a9-a7b2-89aa9355dba5"), callsite("fc696522-75bf-5a4c-9690-67ac8e2fd3d8") }
	end
end
local multiline = where(18, 26, 32, "f4dcb6d8-18b4-594a-a04a-b259eb859214", "where(\n\tundefined,\n\tundefined,\n)")
local ids = { callsite("3c0d8bd6-2f64-55dc-8737-e97c35cc583e"), callsite("77e15477-0d1d-5165-9aac-121ad3dd9075") }
return {
	inFunction = inFunction,
	atRoot = atRoot,
	Holder = Holder,
	multiline = multiline,
	ids = ids,
}
