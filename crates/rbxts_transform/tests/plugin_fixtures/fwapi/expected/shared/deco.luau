-- Compiled with roblox-ts v3.0.0
-- a call inside a parenthesized decorator expression is an ordinary call (the decorator's expression is not a call)
local P1
do
	P1 = setmetatable({}, {
		__tostring = function()
			return "P1"
		end,
	})
	P1.__index = P1
	function P1.new(...)
		local self = setmetatable({}, P1)
		return self:constructor(...) or self
	end
	function P1:constructor()
	end
	do
		P1.__probed = "probe"
	end
	P1 = (probeDecoInject("parenthesized", "injected", nil))(P1) or P1
end
local P2
do
	P2 = setmetatable({}, {
		__tostring = function()
			return "P2"
		end,
	})
	P2.__index = P2
	function P2.new(...)
		local self = setmetatable({}, P2)
		return self:constructor(...) or self
	end
	function P2:constructor()
	end
	do
		P2.__probed = "probe"
	end
	P2 = probeDecoInject("direct", "injected", nil)(P2) or P2
end
local P3
do
	P3 = setmetatable({}, {
		__tostring = function()
			return "P3"
		end,
	})
	P3.__index = P3
	function P3.new(...)
		local self = setmetatable({}, P3)
		return self:constructor(...) or self
	end
	function P3:constructor()
	end
	function P3:method()
	end
	do
		P3.__probed = "probe"
	end
	local _descriptor = (probeDecoInject("method", "injected", nil))(P3, "method", {
		value = P3.method,
	})
	if _descriptor then
		P3.method = _descriptor.value
	end
end
return {
	P1 = P1,
	P2 = P2,
	P3 = P3,
}
