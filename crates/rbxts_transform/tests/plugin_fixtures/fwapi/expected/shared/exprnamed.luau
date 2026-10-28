-- Compiled with roblox-ts v3.0.0
-- Cx::compile_expr_named: a chain compiled as the initializer of `const <name> = <chain>` names its temporary after
-- the variable, as roblox-ts does (`local _target = …`), numbered where the name is taken
local function simple()
	local _target = get("a")
	if _target ~= nil then
		_target = _target.b
	end
	local target = _target
	local r = target
	return r
end
local function taken()
	local _target = get("a")
	if _target ~= nil then
		_target = _target.b
	end
	local target = _target
	do
		local _target_1 = get("b")
		if _target_1 ~= nil then
			_target_1 = _target_1.b
			if _target_1 ~= nil then
				_target_1 = _target_1.c
			end
		end
		local target = _target_1
		local r = target
		print(r)
	end
	return target
end
local function several()
	local _target = get("a")
	if _target ~= nil then
		_target = _target:m()
	end
	local target = _target
	local r1 = target
	local _target_1 = get("b")
	if _target_1 ~= nil then
		_target_1 = _target_1.f
		if _target_1 ~= nil then
			_target_1 = _target_1()
		end
	end
	local target_1 = _target_1
	local r2 = target_1
	local _other = get("c")
	if _other ~= nil then
		_other = _other.b
	end
	local other = _other
	local r3 = other
	return { r1, r2, r3 }
end
local function siblings()
	do
		local _target = get("a")
		if _target ~= nil then
			_target = _target.b
		end
		local target = _target
		local r = target
		print(r)
	end
	do
		local _target = get("b")
		if _target ~= nil then
			_target = _target.b
		end
		local target = _target
		local r = target
		print(r)
	end
end
local function notChain()
	local target = get("x")
	local r = target
	return r
end
local function wrapped()
	local _result = get("a")
	if _result ~= nil then
		_result = _result.b
	end
	local target = _result
	local r1 = target
	local _result_1 = get("b")
	if _result_1 ~= nil then
		_result_1 = _result_1.b
	end
	local other = _result_1
	local r2 = other
	return { r1, r2 }
end
local _target = get("t")
if _target ~= nil then
	_target = _target.b
end
local target = _target
local top = target
return {
	simple = simple,
	taken = taken,
	several = several,
	siblings = siblings,
	notChain = notChain,
	wrapped = wrapped,
	top = top,
}
