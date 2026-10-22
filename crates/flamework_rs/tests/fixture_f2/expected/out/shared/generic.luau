-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local Reflect = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Reflect
local _core = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out")
local Dependency = _core.Dependency
local Flamework = _core.Flamework
local Provider = _core.Provider
local _markers = TS.import(script, game:GetService("ReplicatedStorage"), "TS", "markers")
local conciseOf = _markers.conciseOf
local dependencyOf = _markers.dependencyOf
local either = _markers.either
local idOf = _markers.idOf
local Mark = _markers.Mark
local Registry = _markers.Registry
local second = _markers.second
local Tagged = _markers.Tagged
local textOf = _markers.textOf
local withOptions = _markers.withOptions
local Economy
do
	Economy = setmetatable({}, {
		__tostring = function()
			return "Economy"
		end,
	})
	Economy.__index = Economy
	function Economy.new(...)
		local self = setmetatable({}, Economy)
		return self:constructor(...) or self
	end
	function Economy:constructor()
	end
	do
		-- (Flamework) Economy metadata
		Reflect.defineMetadata(Economy, "identifier", "shared/generic@Economy")
		Reflect.defineMetadata(Economy, "flamework:module", script)
		Reflect.defineMetadata(Economy, "flamework:implements", {})
		Reflect.defineMetadata(Economy, "flamework:parameters", {})
		Reflect.defineMetadata(Economy, "flamework:dependencies", {})
	end
	Economy = Provider()(Economy) or Economy
end
local LoggerImpl
do
	LoggerImpl = setmetatable({}, {
		__tostring = function()
			return "LoggerImpl"
		end,
	})
	LoggerImpl.__index = LoggerImpl
	function LoggerImpl.new(...)
		local self = setmetatable({}, LoggerImpl)
		return self:constructor(...) or self
	end
	function LoggerImpl:constructor()
	end
	function LoggerImpl:log(message)
		print(message)
	end
end
-- A dependency with metadata: what the provider gets is told where it is injected.
local Bank
do
	Bank = setmetatable({}, {
		__tostring = function()
			return "Bank"
		end,
	})
	Bank.__index = Bank
	function Bank.new(...)
		local self = setmetatable({}, Bank)
		return self:constructor(...) or self
	end
	function Bank:constructor(economy, logger)
		self.economy = economy
		self.logger = logger
	end
	do
		-- (Flamework) Bank metadata
		Reflect.defineMetadata(Bank, "identifier", "shared/generic@Bank")
		Reflect.defineMetadata(Bank, "flamework:module", script)
		Reflect.defineMetadata(Bank, "flamework:implements", {})
		Reflect.defineMetadata(Bank, "flamework:parameters", { "shared/generic@Economy", "shared/generic@LoggerImpl" })
		Reflect.defineMetadata(Bank, "flamework:dependencies", { {
			id = "shared/generic@Economy",
		}, {
			id = "shared/generic@LoggerImpl",
			metadata = { 24, "logger", "shared/generic@Shop" },
		} })
	end
	Bank = Provider()(Bank) or Bank
end
-- Generic markers.
local economyId = idOf("shared/generic@Economy")
local shopText = textOf("Shop | undefined")
local economyDependency = dependencyOf({
	id = "shared/generic@Economy",
})
local economyConcise = conciseOf("shared/generic@Economy")
local loggerConcise = conciseOf({
	id = "shared/generic@LoggerImpl",
	metadata = { 33, "logger", "shared/generic@Shop" },
})
local loggerDependency = dependencyOf({
	id = "shared/generic@LoggerImpl",
	metadata = { 34, "logger", "shared/generic@Shop" },
})
-- Core's macros: an inlined id, a rewritten call, dependencies.
local inlined = "shared/generic@Bank"
local inlinedOwn = "[Economy, string]"
local passedThrough = "explicit"
local function isShop(value)
	return Flamework._implements(value, "shared/generic@Shop")
end
local function resolve()
	return { Dependency(nil, "shared/generic@Economy"), Dependency(nil, {
		id = "shared/generic@LoggerImpl",
		metadata = { 44, "logger", "shared/generic@Shop" },
	}), Dependency(nil, idOf("shared/generic@Economy")) }
end
-- Unions, padding, explicit arguments.
local generated = either("shared/generic@Economy")
local explicit = either("mine")
local padded = second(nil, "shared/generic@Economy")
local paddedUndefined = second(nil, "shared/generic@Economy")
local options = withOptions({
	name = "shop",
	size = 3,
}, "shared/generic@Shop")
-- A method, a constructor and a decorator.
local registry = Registry.new():add(1, "shared/generic@Economy"):add(2, "shared/generic@Shop")
local tagged = Tagged.new("shared/generic@Bank")
local Marked
do
	Marked = setmetatable({}, {
		__tostring = function()
			return "Marked"
		end,
	})
	Marked.__index = Marked
	function Marked.new(...)
		local self = setmetatable({}, Marked)
		return self:constructor(...) or self
	end
	function Marked:constructor()
	end
	Marked = Mark("shared/generic@Economy")(Marked) or Marked
end
return {
	Economy = Economy,
	Bank = Bank,
	economyId = economyId,
	shopText = shopText,
	economyDependency = economyDependency,
	economyConcise = economyConcise,
	loggerConcise = loggerConcise,
	loggerDependency = loggerDependency,
	inlined = inlined,
	inlinedOwn = inlinedOwn,
	passedThrough = passedThrough,
	isShop = isShop,
	resolve = resolve,
	generated = generated,
	explicit = explicit,
	padded = padded,
	paddedUndefined = paddedUndefined,
	options = options,
	registry = registry,
	tagged = tagged,
	Marked = Marked,
}
