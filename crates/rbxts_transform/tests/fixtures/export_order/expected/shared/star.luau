-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("include"):WaitForChild("RuntimeLib"))
local exports = {}
-- `export *`: the module's own exports first (functions first), then the names the star brings, in its order.
local own = 1
for _k, _v in TS.import(script, game:GetService("ReplicatedStorage"), "shared", "starred") or {} do
	exports[_k] = _v
end
local function ownFn()
	return own
end
exports.ownFn = ownFn
exports.own = own
return exports
