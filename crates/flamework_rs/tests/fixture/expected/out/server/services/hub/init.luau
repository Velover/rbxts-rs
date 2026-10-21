-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local Reflect = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Reflect
local _core = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out")
local Injectable = _core.Injectable
local Provider = _core.Provider
-- `index` files give `init` ids.
local Hub
do
	Hub = setmetatable({}, {
		__tostring = function()
			return "Hub"
		end,
	})
	Hub.__index = Hub
	function Hub.new(...)
		local self = setmetatable({}, Hub)
		return self:constructor(...) or self
	end
	function Hub:constructor()
	end
	do
		-- (Flamework) Hub metadata
		Reflect.defineMetadata(Hub, "identifier", "server/services/hub/init@Hub")
		Reflect.defineMetadata(Hub, "flamework:module", script)
		Reflect.defineMetadata(Hub, "flamework:implements", {})
		Reflect.defineMetadata(Hub, "flamework:parameters", {})
		Reflect.defineMetadata(Hub, "flamework:dependencies", {})
	end
	Hub = Provider()(Hub) or Hub
end
-- Constructed through a module, not registered.
local Helper
do
	Helper = setmetatable({}, {
		__tostring = function()
			return "Helper"
		end,
	})
	Helper.__index = Helper
	function Helper.new(...)
		local self = setmetatable({}, Helper)
		return self:constructor(...) or self
	end
	function Helper:constructor(hub)
		self.hub = hub
	end
	do
		-- (Flamework) Helper metadata
		Reflect.defineMetadata(Helper, "identifier", "server/services/hub/init@Helper")
		Reflect.defineMetadata(Helper, "flamework:module", script)
		Reflect.defineMetadata(Helper, "flamework:implements", {})
		Reflect.defineMetadata(Helper, "flamework:parameters", { "server/services/hub/init@Hub" })
		Reflect.defineMetadata(Helper, "flamework:dependencies", { {
			id = "server/services/hub/init@Hub",
		} })
	end
	Helper = Injectable()(Helper) or Helper
end
return {
	Hub = Hub,
	Helper = Helper,
}
