-- Compiled with roblox-ts v3.0.0
-- More of the outer offer (`calls_chain_outer`): a call TypeScript counts in an optional chain that roblox-ts's
-- `flattenOptionalChain` cuts at a `!`; a `Callee::Method` replacement, with and without the call's own `?.`; and a
-- send as Flamework's networking makes it, which reads the chain's base, runs the arguments, then reads the receiver
-- chain again (`compile_call_with` with deferred slots).
-- sets `box.ev` to undefined
-- through a `!`
local n1 = "inlined"
local n2 = Rewrite.impl(sideEffect())
local _n3 = Rewrite.impl
if _n3 ~= nil then
	_n3 = _n3(sideEffect())
end
local n3 = _n3
local n4 = "inlined"
-- declined through a `!`: lowered as usual
local _result = get()
if _result ~= nil then
	_result = _result.inner
end
local n5 = _result:other(sideEffect())
-- Callee::Method
local m1 = Probed:impl(sideEffect())
local _self = Probed
local _m2 = _self.impl
if _m2 ~= nil then
	_m2 = _m2(_self, sideEffect())
end
local m2 = _m2
local _self_1 = Probed
local _m3 = _self_1.impl
if _m3 ~= nil then
	_m3 = _m3(_self_1, 1)
end
local m3 = _m3
local function methods()
	local _self_2 = Probed
	local _result_1 = _self_2.impl
	if _result_1 ~= nil then
		_result_1(_self_2, 2)
	end
	Probed:impl(3)
end
-- a send: the base, the arguments, the receiver again
local function send(make)
	local _fn = Send.impl
	local _exp = box.ev
	local _exp_1 = clearBox() + make()
	local _result_1 = box.ev
	if _result_1 ~= nil then
		_result_1 = _result_1.inner
	end
	_fn(_exp, _exp_1, _result_1)
	local _fn_1 = Send.impl
	local _exp_2 = box.ev
	local _exp_3 = sideEffect()
	local _result_2 = box.ev
	if _result_2 ~= nil then
		_result_2 = _result_2.inner
	end
	local sent = _fn_1(_exp_2, _exp_3, _result_2)
	-- declined: lowered as usual
	local _result_3 = box.ev
	if _result_3 ~= nil then
		_result_3 = _result_3.inner:other(clearBox() + make())
	end
	return sent
end
return {
	methods = methods,
	send = send,
	n1 = n1,
	n2 = n2,
	n3 = n3,
	n4 = n4,
	n5 = n5,
	m1 = m1,
	m2 = m2,
	m3 = m3,
}
