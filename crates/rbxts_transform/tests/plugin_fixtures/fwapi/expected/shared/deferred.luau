-- Compiled with roblox-ts v3.0.0
-- compile_call_with: a value the plugin builds in the second slot (here the third written argument, compiled there),
-- so its temporaries are named between the first and the last argument's, and the earlier values are bound to locals
-- as for a written argument with prereqs (`ensureTransformOrder`)
local _condition = x
if _condition == nil then
	_condition = f(1)
end
local _condition_1 = z
if _condition_1 == nil then
	_condition_1 = f(3)
end
local _condition_2 = y
if _condition_2 == nil then
	_condition_2 = f(2)
end
local d = probeDeferred(_condition, _condition_1, _condition_2)
local _exp = f(4)
local _condition_3 = z
if _condition_3 == nil then
	_condition_3 = f(6)
end
local e = probeDeferred(_exp, _condition_3, f(5))
return {
	d = d,
	e = e,
}
