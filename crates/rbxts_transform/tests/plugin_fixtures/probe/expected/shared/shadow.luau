-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("include"):WaitForChild("RuntimeLib"))
local _impl = TS.import(script, game:GetService("ReplicatedStorage"), "shared", "runtime").impl
local _runtime = TS.import(script, game:GetService("ReplicatedStorage"), "shared", "runtime")
local impl = _runtime.impl
local other = _runtime.other
local a = impl("a", other)
-- `impl` would mean the parameter or the local here: the import is not reused
local function shadowed(impl)
	return _impl(impl, 3)
end
local function withLocal()
	local impl = "x"
	return _impl(impl, 4)
end
return {
	a = a,
	shadowed = shadowed,
	withLocal = withLocal,
}
