-- Compiled with roblox-ts v3.0.0
-- Cx::ordered_element_call: `o[k](o, args)` built from compiled parts, as roblox-ts orders an element call
local _fn = o[k]
local _condition = x
if _condition == nil then
	_condition = f(1)
end
local c1 = _fn(o, _condition, f(2))
local _self = get()
local _condition_1 = x
if _condition_1 == nil then
	_condition_1 = 4
end
local c2 = _self.m(_self, f(3), _condition_1)
local c3 = o[key()](o, 5, 6)
local _fn_1 = o[k]
local _exp = f(7)
k = "n"
local _ = k
local c4 = _fn_1(o, _exp, 8)
return {
	c1 = c1,
	c2 = c2,
	c3 = c3,
	c4 = c4,
}
