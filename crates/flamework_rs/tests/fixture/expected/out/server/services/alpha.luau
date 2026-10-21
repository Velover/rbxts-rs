-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local Reflect = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Reflect
local Provider = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Provider
-- A provider with dependencies and lifecycle interfaces.
local Alpha
do
	Alpha = setmetatable({}, {
		__tostring = function()
			return "Alpha"
		end,
	})
	Alpha.__index = Alpha
	function Alpha.new(...)
		local self = setmetatable({}, Alpha)
		return self:constructor(...) or self
	end
	function Alpha:constructor(beta, hub)
		self.beta = beta
		self.hub = hub
	end
	function Alpha:onInit()
	end
	function Alpha:onStart()
		print(self.beta, self.hub)
	end
	do
		-- (Flamework) Alpha metadata
		Reflect.defineMetadata(Alpha, "identifier", "server/services/alpha@Alpha")
		Reflect.defineMetadata(Alpha, "flamework:module", script)
		Reflect.defineMetadata(Alpha, "flamework:implements", { "$:lifecycle/lifecycleInterfaces@OnStart", "$:lifecycle/lifecycleInterfaces@OnInit" })
		Reflect.defineMetadata(Alpha, "flamework:parameters", { "server/services/beta@Beta", "server/services/hub/init@Hub" })
		Reflect.defineMetadata(Alpha, "flamework:dependencies", { {
			id = "server/services/beta@Beta",
		}, {
			id = "server/services/hub/init@Hub",
		} })
	end
	Alpha = Provider()(Alpha) or Alpha
end
return {
	Alpha = Alpha,
}
