-- Compiled with roblox-ts v3.0.0
-- compile_object_literal_with: the literal a TS transformer rebuilt as `{ ...others, w: extra, moved: value }`, the
-- generated values built at their places in the emitted order (`replace_deferred`, `append_deferred`), so their
-- statements run after the fields before them: `local _left = "w"`, the statements, `_object[_left] = value`
local j = 0
local plain = {
	a = 1,
	w = 3,
	moved = 2,
}
local _object = {
	a = g(1),
	b = g(3),
	w = g(6),
}
local _left = "moved"
local _original = j
j += 1
_object[_left] = if _original > 0 then g(4) else g(5)
local valueStatements = _object
local _object_1 = {
	a = g(1),
	[g(2)] = 3,
}
local _left_1 = "w"
local _original_1 = j
j += 1
_object_1[_left_1] = if _original_1 > 1 then g(7) else g(8)
local _left_2 = "moved"
local _original_2 = j
j += 1
_object_1[_left_2] = if _original_2 > 0 then g(4) else g(5)
local both = _object_1
local _object_2 = {
	a = g(1),
	w = g(7),
}
local _left_3 = "moved"
local _object_3 = {}
local _left_4 = "x"
local _original_3 = j
j += 1
_object_3[_left_4] = if _original_3 > 0 then g(4) else g(5)
_object_3.y = g(6)
_object_2[_left_3] = _object_3
local nested = _object_2
local _object_4 = {
	a = g(1),
}
local _left_5 = "w"
local _original_4 = j
j += 1
_object_4[_left_5] = if _original_4 > 0 then g(3) else g(4)
_object_4.moved = g(2)
local extraOnly = _object_4
return {
	plain = plain,
	valueStatements = valueStatements,
	both = both,
	nested = nested,
	extraOnly = extraOnly,
}
