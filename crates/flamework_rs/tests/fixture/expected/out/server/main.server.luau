-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local _core = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out")
local Flamework = _core.Flamework
local requireModules = _core.requireModules
local loaded = requireModules("src/server/services", { "ServerScriptService", "TS", "services" })
Flamework.createModule():registerProviders("src/server/services", nil, { "ServerScriptService", "TS", "services" }):registerProvidersGlob("src/server/services/**/*.ts", nil, "src/server/services/**/*.ts"):registerProvidersGlob("./services/hub/*.ts", nil, "src/server/services/hub/*.ts")
-- F2: a generic id, declined in F1 and compiled as a plain call.
print(loaded, "$p:string")
-- A glob that matches nothing, and a path with no module in it, are warned about.
Flamework.createModule():registerProvidersGlob("src/server/nothing/*.ts", nil, "src/server/nothing/*.ts")
