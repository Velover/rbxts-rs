-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local t = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@rbxts", "t", "lib", "ts").t
local Flamework = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Flamework
-- Arrays, tuples, maps, sets, records and index signatures.
local numbers = Flamework.createGuard(t.array(t.number))
local readonlyNumbers = Flamework.createGuard(t.array(t.number))
local nested = Flamework.createGuard(t.array(t.array(t.string)))
local pair = Flamework.createGuard(t.strictArray(t.number, t.string))
local optionalPair = Flamework.createGuard(t.strictArray(t.number, t.optional(t.string)))
local labeled = Flamework.createGuard(t.strictArray(t.number, t.optional(t.string)))
local rest = Flamework.createGuard((function(element, rest_1)
	return function(value)
		if not (type(value) == "table") then
			return false
		end
		local size = 0
		for key_1 in pairs(value) do
			if not (type(key_1) == "number") or key_1 < 1 or key_1 % 1 ~= 0 then
				return false
			end
			if key_1 > size then
				size = key_1
			end
		end
		if size < 1 then
			return false
		end
		local list = value
		if not element(list[1]) then
			return false
		end
		for index_1 = 2, size do
			if not rest_1(list[index_1]) then
				return false
			end
		end
		return true
	end
end)(t.number, t.string))
local restOnly = Flamework.createGuard(t.array(t.string))
local middleRest = Flamework.createGuard((function(element_1, rest_2, last)
	return function(value_1)
		if not (type(value_1) == "table") then
			return false
		end
		local size_1 = 0
		for key_2 in pairs(value_1) do
			if not (type(key_2) == "number") or key_2 < 1 or key_2 % 1 ~= 0 then
				return false
			end
			if key_2 > size_1 then
				size_1 = key_2
			end
		end
		if size_1 < 2 then
			return false
		end
		local list_1 = value_1
		if not element_1(list_1[1]) then
			return false
		end
		for index_2 = 2, size_1 - 1 do
			if not rest_2(list_1[index_2]) then
				return false
			end
		end
		if not last(list_1[size_1]) then
			return false
		end
		return true
	end
end)(t.number, t.string, t.boolean))
local twoTrailing = Flamework.createGuard((function(element_2, rest_3, last_1, last_2)
	return function(value_2)
		if not (type(value_2) == "table") then
			return false
		end
		local size_2 = 0
		for key_3 in pairs(value_2) do
			if not (type(key_3) == "number") or key_3 < 1 or key_3 % 1 ~= 0 then
				return false
			end
			if key_3 > size_2 then
				size_2 = key_3
			end
		end
		if size_2 < 3 then
			return false
		end
		local list_2 = value_2
		if not element_2(list_2[1]) then
			return false
		end
		for index_3 = 2, size_2 - 2 do
			if not rest_3(list_2[index_3]) then
				return false
			end
		end
		if not last_1(list_2[size_2 - 1]) then
			return false
		end
		if not last_2(list_2[size_2]) then
			return false
		end
		return true
	end
end)(t.number, t.string, t.boolean, t.number))
local optionalRest = Flamework.createGuard((function(element_3, element_4, rest_4)
	return function(value_3)
		if not (type(value_3) == "table") then
			return false
		end
		local size_3 = 0
		for key_4 in pairs(value_3) do
			if not (type(key_4) == "number") or key_4 < 1 or key_4 % 1 ~= 0 then
				return false
			end
			if key_4 > size_3 then
				size_3 = key_4
			end
		end
		if size_3 < 1 then
			return false
		end
		local list_3 = value_3
		if not element_3(list_3[1]) then
			return false
		end
		if not element_4(list_3[2]) then
			return false
		end
		for index_4 = 3, size_3 do
			if not rest_4(list_3[index_4]) then
				return false
			end
		end
		return true
	end
end)(t.number, t.optional(t.string), t.boolean))
local readonlyTuple = Flamework.createGuard(t.strictArray(t.number, t.number))
local emptyTuple = Flamework.createGuard(t.strictArray())
local map = Flamework.createGuard(t.map(t.string, t.number))
local readonlyMap = Flamework.createGuard(t.map(t.number, t.array(t.string)))
local set = Flamework.createGuard(t.set(t.string))
local readonlySet = Flamework.createGuard(t.set(t.Vector3))
local record = Flamework.createGuard(t.map(t.string, t.number))
local keyedRecord = Flamework.createGuard(t.interface({
	a = t.number,
	b = t.number,
}))
local numericRecord = Flamework.createGuard(t.interface({
	[10] = t.string,
	[2] = t.string,
}))
local indexString = Flamework.createGuard(t.map(t.string, t.boolean))
local indexNumber = Flamework.createGuard(t.map(t.number, t.string))
local indexAndProps = Flamework.createGuard(t.intersection(t.interface({
	name = t.string,
}), t.map(t.string, t.string)))
local partialRecord = Flamework.createGuard(t.interface({
	x = t.optional(t.number),
	y = t.optional(t.number),
}))
return {
	numbers = numbers,
	readonlyNumbers = readonlyNumbers,
	nested = nested,
	pair = pair,
	optionalPair = optionalPair,
	labeled = labeled,
	rest = rest,
	restOnly = restOnly,
	middleRest = middleRest,
	twoTrailing = twoTrailing,
	optionalRest = optionalRest,
	readonlyTuple = readonlyTuple,
	emptyTuple = emptyTuple,
	map = map,
	readonlyMap = readonlyMap,
	set = set,
	readonlySet = readonlySet,
	record = record,
	keyedRecord = keyedRecord,
	numericRecord = numericRecord,
	indexString = indexString,
	indexNumber = indexNumber,
	indexAndProps = indexAndProps,
	partialRecord = partialRecord,
}
