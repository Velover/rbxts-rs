-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local Reflect = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Reflect
--[[
	A user decorator that asks for more metadata than the core decorators do.

	@metadata reflect identifier flamework:parameters flamework:parameter_names flamework:implements
]]
local function Tracked()
	return function(ctor)
		Reflect.defineMetadata(ctor, "tracked", true)
	end
end
-- Requested per member. @metadata reflect flamework:type flamework:return_type flamework:parameters
local function Field()
	return function(target, key)
		print(target, key)
	end
end
local Measured
do
	Measured = setmetatable({}, {
		__tostring = function()
			return "Measured"
		end,
	})
	Measured.__index = Measured
	function Measured.new(...)
		local self = setmetatable({}, Measured)
		return self:constructor(...) or self
	end
	function Measured:constructor(count, generic, anything, _param)
		local size = _param.size
		self.count = count
		self.generic = generic
		self.anything = anything
		self.size = 0
		self.shape = {
			size = 1,
		}
		print(size)
	end
	function Measured:measure(by)
		return {
			size = by.size,
		}
	end
	do
		-- (Flamework) Measured metadata
		Reflect.defineMetadata(Measured, "identifier", "server/reflected@Measured")
		Reflect.defineMetadata(Measured, "flamework:module", script)
		Reflect.defineMetadata(Measured, "flamework:implements", { "server/reflected@Shape" })
		Reflect.defineMetadata(Measured, "flamework:parameters", { "$p:number", "$tp:T", "@rbxts/compiler-types:types/core@defined", "server/reflected@Shape" })
		Reflect.defineMetadata(Measured, "flamework:parameter_names", { "count", "generic", "anything", "_binding_" })
		-- (Flamework) Measured.shape metadata
		Reflect.defineMetadata(Measured, "flamework:type", "server/reflected@Shape", "shape")
		-- (Flamework) Measured.measure metadata
		Reflect.defineMetadata(Measured, "flamework:parameters", { "server/reflected@Measured" }, "measure")
		Reflect.defineMetadata(Measured, "flamework:return_type", "server/reflected@Shape", "measure")
	end
	Field()(Measured, "shape")
	local _descriptor = Field()(Measured, "measure", {
		value = Measured.measure,
	})
	if _descriptor then
		Measured.measure = _descriptor.value
	end
	Measured = Tracked()(Measured) or Measured
end
return {
	Tracked = Tracked,
	Field = Field,
	Measured = Measured,
}
