-- Compiled with roblox-ts v3.0.0
-- compile_object_literal with FieldOrder::Emitted: the literal as a TS transformer rebuilt it, members reversed and
-- `w = 4` appended first, evaluated in that order (a table built inline until a member needs statements)
local i = 0
local plain = {
	w = 4,
	c = true,
	b = "x",
	a = 1,
}
local _object = {
	w = 4,
	m = function(self)
		return f(4)
	end,
}
for _k, _v in {
	s = f(3),
} do
	_object[_k] = _v
end
_object.b = f(2)
_object.a = f(1)
local effects = _object
local _object_1 = {
	w = 4,
	[f(5)] = 6,
	c = f(4),
}
local _left = "b"
local _original = i
i += 1
_object_1[_left] = if _original > 0 then f(2) else f(3)
_object_1.a = f(1)
local statements = _object_1
return {
	plain = plain,
	effects = effects,
	statements = statements,
}
