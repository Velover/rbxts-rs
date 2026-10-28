-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("include"):WaitForChild("RuntimeLib"))
-- import_named_at: a member's import asked for at its class, so before the decorator's (TypeScript transforms a
-- class's members before its decorators), though the decorator is written first
local _decos = TS.import(script, game:GetService("ReplicatedStorage"), "shared", "decos")
local dB = _decos.dB
local dA = _decos.dA
local C
do
	C = setmetatable({}, {
		__tostring = function()
			return "C"
		end,
	})
	C.__index = C
	function C.new(...)
		local self = setmetatable({}, C)
		return self:constructor(...) or self
	end
	function C:constructor()
	end
	function C:m()
		return dB
	end
	do
		C.__probed = "probe"
	end
	C = dA(C) or C
end
return {
	C = C,
}
