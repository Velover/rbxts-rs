-- Compiled with roblox-ts v3.0.0
-- The outermost call of an optional chain offered before the chain is compiled (`calls_chain_outer`): a value taken
-- replaces the whole chain (nothing of it is evaluated), a replaced callee keeps only the call's own `?.`.
-- o?.m()
local i1 = "inlined"
local r1 = Rewrite.impl(sideEffect())
-- o.m?.()
local i2 = "inlined"
local _r2 = Rewrite.impl
if _r2 ~= nil then
	_r2 = _r2(sideEffect())
end
local r2 = _r2
-- o?.m?.()
local i3 = "inlined"
local _r3 = Rewrite.impl
if _r3 ~= nil then
	_r3 = _r3(sideEffect())
end
local r3 = _r3
-- deeper chains
local i4 = "inlined"
local r4 = Rewrite.impl(1)
local function statements()
	local _ = "inlined"
	local _result = Rewrite.impl
	if _result ~= nil then
		_result(sideEffect())
	end
	Rewrite.impl(2)
end
-- declined: lowered as usual
local _d1 = maybe
if _d1 ~= nil then
	_d1 = _d1:other(1)
end
local d1 = _d1
local _d2 = maybe
if _d2 ~= nil then
	local _self = _d2.inner
	_d2 = _self.other
	if _d2 ~= nil then
		_d2 = _d2(_self, 2)
	end
end
local d2 = _d2
return {
	statements = statements,
	i1 = i1,
	r1 = r1,
	i2 = i2,
	r2 = r2,
	i3 = i3,
	r3 = r3,
	i4 = i4,
	r4 = r4,
	d1 = d1,
	d2 = d2,
}
