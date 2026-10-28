-- Compiled with roblox-ts v3.0.0
-- a property key answer with the element access spelling (`KeyAnswer::as_element_access`): a method call through it
-- is roblox-ts's element call, `obj.key(obj, …)`, as for the `obj["key"](…)` a TS transformer writes
local a = ek.k_elem(ek, 1)
local b = ek.elemsame(ek, 2)
local _self = getEk()
local c = _self.k_elem(_self, 3)
local function d(o)
	return o.k_elem(o, o.elemsame(o, 4))
end
return {
	d = d,
	a = a,
	b = b,
	c = c,
}
