-- Compiled with roblox-ts v3.0.0
-- ObjectEdits::drop: a written member left out of the literal a TS transformer rebuilt, an accessor included (no
-- "Getters and Setters are not supported!"), and `attributes` appended in its stead with a deferred value
local k = 0
local getter = {
	tag = "x",
	attributes = {
		name = h(1),
	},
	w = 4,
}
local _object = {
	tag = h(1),
}
local _left = "attributes"
local _original = k
k += 1
_object[_left] = if _original > 0 then h(2) else h(3)
_object.w = 4
local setter = _object
local field = {
	a = h(1),
	b = h(3),
	attributes = h(4),
	w = 4,
}
local _object_1 = {
	a = h(1),
}
local _left_1 = "attributes"
local _object_2 = {}
local _left_2 = "x"
local _original_1 = k
k += 1
_object_2[_left_2] = if _original_1 > 0 then h(2) else h(3)
_object_1[_left_1] = _object_2
_object_1.w = 4
local method = _object_1
local _object_3 = table.clone({
	s = h(1),
})
setmetatable(_object_3, nil)
_object_3.b = h(2)
_object_3.attributes = h(3)
_object_3.w = 4
local spread = _object_3
return {
	getter = getter,
	setter = setter,
	field = field,
	method = method,
	spread = spread,
}
