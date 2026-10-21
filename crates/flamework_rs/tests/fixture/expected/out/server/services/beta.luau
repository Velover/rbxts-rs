-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local Reflect = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Reflect
local Provider = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Provider
local Beta
do
	Beta = setmetatable({}, {
		__tostring = function()
			return "Beta"
		end,
	})
	Beta.__index = Beta
	function Beta.new(...)
		local self = setmetatable({}, Beta)
		return self:constructor(...) or self
	end
	function Beta:constructor()
		self.value = 1
	end
	Beta.label = "beta"
	do
		-- (Flamework) Beta metadata
		Reflect.defineMetadata(Beta, "identifier", "server/services/beta@Beta")
		Reflect.defineMetadata(Beta, "flamework:module", script)
		Reflect.defineMetadata(Beta, "flamework:implements", {})
		Reflect.defineMetadata(Beta, "flamework:parameters", {})
		Reflect.defineMetadata(Beta, "flamework:dependencies", {})
	end
	Beta = Provider({
		loadOrder = 1,
	})(Beta) or Beta
end
return {
	Beta = Beta,
}
