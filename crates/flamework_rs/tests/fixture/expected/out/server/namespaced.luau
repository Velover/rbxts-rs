-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local Reflect = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Reflect
local Provider = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Provider
local Outer = {}
do
	local _container = Outer
	local Inner = {}
	do
		local _container_1 = Inner
		-- A provider in a namespace: its id names the namespaces, and it is still module-level.
		local Deep
		do
			Deep = setmetatable({}, {
				__tostring = function()
					return "Deep"
				end,
			})
			Deep.__index = Deep
			function Deep.new(...)
				local self = setmetatable({}, Deep)
				return self:constructor(...) or self
			end
			function Deep:constructor()
			end
			do
				-- (Flamework) Deep metadata
				Reflect.defineMetadata(Deep, "identifier", "server/namespaced@Outer.Inner.Deep")
				Reflect.defineMetadata(Deep, "flamework:module", script)
				Reflect.defineMetadata(Deep, "flamework:implements", {})
				Reflect.defineMetadata(Deep, "flamework:parameters", {})
				Reflect.defineMetadata(Deep, "flamework:dependencies", {})
			end
			Deep = Provider()(Deep) or Deep
		end
		_container_1.Deep = Deep
	end
	_container.Inner = Inner
end
-- A provider made by a function call is not module-level: no `flamework:module`.
local function makeProvider()
	local Local
	do
		Local = setmetatable({}, {
			__tostring = function()
				return "Local"
			end,
		})
		Local.__index = Local
		function Local.new(...)
			local self = setmetatable({}, Local)
			return self:constructor(...) or self
		end
		function Local:constructor()
		end
		do
			-- (Flamework) Local metadata
			Reflect.defineMetadata(Local, "identifier", "server/namespaced@makeProvider.Local")
			Reflect.defineMetadata(Local, "flamework:implements", {})
			Reflect.defineMetadata(Local, "flamework:parameters", {})
			Reflect.defineMetadata(Local, "flamework:dependencies", {})
		end
		Local = Provider()(Local) or Local
	end
	return Local
end
return {
	Outer = Outer,
	makeProvider = makeProvider,
}
