-- Compiled with roblox-ts v3.0.0
-- computed keys: offered to property_key with the key of a single string literal type, else `None` (declined)
local ka = "a"
local km = "m"
local c1 = keyed.k_a
keyName()
local c2 = keyed.k_b
keyed.k_a = 1
keyName()
keyed.k_b += 2
local c3 = keyed:k_m(1)
keyName()
local c4 = keyed:k_m(2)
local _self = keyed
local _c5 = _self.k_m
if _c5 ~= nil then
	_c5 = _c5(_self, 3)
end
local c5 = _c5
local _c6 = maybeKeyed
if _c6 ~= nil then
	_c6 = _c6.k_a
end
local c6 = _c6
local c7 = keyed[anyKey]
local c8 = keyedList[0]
keyName()
keyed.k_opt = nil
local c9 = "marked=marked:-;marked=marked:the marked one;maybeMarked=maybeMarked:piped text;keyed=keyed:-"
-- assignments with a computed key: the hook compiles `key_node`
local CompKeys
do
	CompKeys = setmetatable({}, {
		__tostring = function()
			return "CompKeys"
		end,
	})
	CompKeys.__index = CompKeys
	function CompKeys.new(...)
		local self = setmetatable({}, CompKeys)
		return self:constructor(...) or self
	end
	function CompKeys:constructor()
		self.attrs = {
			count = 0,
			label = "",
		}
	end
	function CompKeys:run(k)
		local kc = "count"
		self:setAttribute("count", 2)
		self:setAttribute(k, 1)
	end
end
return {
	c1 = c1,
	c2 = c2,
	c3 = c3,
	c4 = c4,
	c5 = c5,
	c6 = c6,
	c7 = c7,
	c8 = c8,
	c9 = c9,
	CompKeys = CompKeys,
}
