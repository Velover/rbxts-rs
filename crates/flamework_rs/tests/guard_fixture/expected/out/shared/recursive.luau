-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local t = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@rbxts", "t", "lib", "ts").t
local Flamework = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Flamework
-- Recursive types (the specs' recursiveGuards types), checked through functions that call themselves.
-- A union whose members both go into the same children.
-- The same without a discriminant.
local tree = Flamework.createGuard((function()
	local depth = 0
	local memo
	local Tree_1
	Tree_1 = t.interface({
		value = t.number,
		kids = t.array(function(value_1)
			if depth >= 1000 then
				return false
			end
			local results
			if type(value_1) == "table" then
				results = memo[Tree_1]
				if results == nil then
					results = {}
					memo[Tree_1] = results
				else
					local known = results[value_1]
					if known ~= nil then
						return known
					end
				end
			end
			depth += 1
			local result = Tree_1(value_1)
			depth -= 1
			if results ~= nil then
				results[value_1] = result
			end
			return result
		end),
	})
	return function(value_2)
		local outerDepth = depth
		local outerMemo = memo
		memo = {}
		local ok, result_1 = pcall(Tree_1, value_2)
		depth = outerDepth
		memo = outerMemo
		if not ok then
			error(result_1, 0)
		end
		return result_1
	end
end)())
local chain = Flamework.createGuard((function()
	local depth_1 = 0
	local memo_1
	local Chain_1
	Chain_1 = t.interface({
		value = t.number,
		next = t.optional(function(value_3)
			if depth_1 >= 1000 then
				return false
			end
			local results_1
			if type(value_3) == "table" then
				results_1 = memo_1[Chain_1]
				if results_1 == nil then
					results_1 = {}
					memo_1[Chain_1] = results_1
				else
					local known_1 = results_1[value_3]
					if known_1 ~= nil then
						return known_1
					end
				end
			end
			depth_1 += 1
			local result_2 = Chain_1(value_3)
			depth_1 -= 1
			if results_1 ~= nil then
				results_1[value_3] = result_2
			end
			return result_2
		end),
	})
	return function(value_4)
		local outerDepth_1 = depth_1
		local outerMemo_1 = memo_1
		memo_1 = {}
		local ok_1, result_3 = pcall(Chain_1, value_4)
		depth_1 = outerDepth_1
		memo_1 = outerMemo_1
		if not ok_1 then
			error(result_3, 0)
		end
		return result_3
	end
end)())
local menu = Flamework.createGuard((function()
	local depth_2 = 0
	local memo_2
	local Menu_1
	Menu_1 = t.interface({
		label = t.string,
		items = t.map(t.string, function(value_5)
			if depth_2 >= 1000 then
				return false
			end
			local results_2
			if type(value_5) == "table" then
				results_2 = memo_2[Menu_1]
				if results_2 == nil then
					results_2 = {}
					memo_2[Menu_1] = results_2
				else
					local known_2 = results_2[value_5]
					if known_2 ~= nil then
						return known_2
					end
				end
			end
			depth_2 += 1
			local result_4 = Menu_1(value_5)
			depth_2 -= 1
			if results_2 ~= nil then
				results_2[value_5] = result_4
			end
			return result_4
		end),
	})
	return function(value_6)
		local outerDepth_2 = depth_2
		local outerMemo_2 = memo_2
		memo_2 = {}
		local ok_2, result_5 = pcall(Menu_1, value_6)
		depth_2 = outerDepth_2
		memo_2 = outerMemo_2
		if not ok_2 then
			error(result_5, 0)
		end
		return result_5
	end
end)())
local json = Flamework.createGuard((function()
	local depth_3 = 0
	local memo_3
	local Json_1 = (function()
		local Json_2
		Json_2 = t.unionList({ t.boolean, t.string, t.number, t.array(function(value_7)
			if depth_3 >= 1000 then
				return false
			end
			local results_3
			if type(value_7) == "table" then
				results_3 = memo_3[Json_2]
				if results_3 == nil then
					results_3 = {}
					memo_3[Json_2] = results_3
				else
					local known_3 = results_3[value_7]
					if known_3 ~= nil then
						return known_3
					end
				end
			end
			depth_3 += 1
			local result_6 = Json_2(value_7)
			depth_3 -= 1
			if results_3 ~= nil then
				results_3[value_7] = result_6
			end
			return result_6
		end), t.map(t.string, function(value_8)
			if depth_3 >= 1000 then
				return false
			end
			local results_4
			if type(value_8) == "table" then
				results_4 = memo_3[Json_2]
				if results_4 == nil then
					results_4 = {}
					memo_3[Json_2] = results_4
				else
					local known_4 = results_4[value_8]
					if known_4 ~= nil then
						return known_4
					end
				end
			end
			depth_3 += 1
			local result_7 = Json_2(value_8)
			depth_3 -= 1
			if results_4 ~= nil then
				results_4[value_8] = result_7
			end
			return result_7
		end) })
		return Json_2
	end)()
	return function(value_9)
		local outerDepth_3 = depth_3
		local outerMemo_3 = memo_3
		memo_3 = {}
		local ok_3, result_8 = pcall(Json_1, value_9)
		depth_3 = outerDepth_3
		memo_3 = outerMemo_3
		if not ok_3 then
			error(result_8, 0)
		end
		return result_8
	end
end)())
local directory = Flamework.createGuard((function()
	local depth_4 = 0
	local memo_4
	local Directory_1
	Directory_1 = t.interface({
		name = t.string,
		files = t.optional(t.array(t.interface({
			name = t.string,
			parent = function(value_10)
				if depth_4 >= 1000 then
					return false
				end
				local results_5
				if type(value_10) == "table" then
					results_5 = memo_4[Directory_1]
					if results_5 == nil then
						results_5 = {}
						memo_4[Directory_1] = results_5
					else
						local known_5 = results_5[value_10]
						if known_5 ~= nil then
							return known_5
						end
					end
				end
				depth_4 += 1
				local result_9 = Directory_1(value_10)
				depth_4 -= 1
				if results_5 ~= nil then
					results_5[value_10] = result_9
				end
				return result_9
			end,
		}))),
	})
	return function(value_11)
		local outerDepth_4 = depth_4
		local outerMemo_4 = memo_4
		memo_4 = {}
		local ok_4, result_10 = pcall(Directory_1, value_11)
		depth_4 = outerDepth_4
		memo_4 = outerMemo_4
		if not ok_4 then
			error(result_10, 0)
		end
		return result_10
	end
end)())
local node = Flamework.createGuard((function()
	local depth_5 = 0
	local memo_5
	local Node_1 = (function()
		local Node_2
		Node_2 = t.union(t.interface({
			kind = t.literal("folder"),
			children = t.optional(t.array(function(value_12)
				if depth_5 >= 1000 then
					return false
				end
				local results_6
				if type(value_12) == "table" then
					results_6 = memo_5[Node_2]
					if results_6 == nil then
						results_6 = {}
						memo_5[Node_2] = results_6
					else
						local known_6 = results_6[value_12]
						if known_6 ~= nil then
							return known_6
						end
					end
				end
				depth_5 += 1
				local result_11 = Node_2(value_12)
				depth_5 -= 1
				if results_6 ~= nil then
					results_6[value_12] = result_11
				end
				return result_11
			end)),
		}), t.interface({
			kind = t.literal("group"),
			children = t.optional(t.array(function(value_13)
				if depth_5 >= 1000 then
					return false
				end
				local results_7
				if type(value_13) == "table" then
					results_7 = memo_5[Node_2]
					if results_7 == nil then
						results_7 = {}
						memo_5[Node_2] = results_7
					else
						local known_7 = results_7[value_13]
						if known_7 ~= nil then
							return known_7
						end
					end
				end
				depth_5 += 1
				local result_12 = Node_2(value_13)
				depth_5 -= 1
				if results_7 ~= nil then
					results_7[value_13] = result_12
				end
				return result_12
			end)),
		}))
		return Node_2
	end)()
	return function(value_14)
		local outerDepth_5 = depth_5
		local outerMemo_5 = memo_5
		memo_5 = {}
		local ok_5, result_13 = pcall(Node_1, value_14)
		depth_5 = outerDepth_5
		memo_5 = outerMemo_5
		if not ok_5 then
			error(result_13, 0)
		end
		return result_13
	end
end)())
local either = Flamework.createGuard((function()
	local depth_6 = 0
	local memo_6
	local Either_1 = (function()
		local Either_2
		Either_2 = t.union(t.interface({
			x = t.number,
			kids = t.array(function(value_15)
				if depth_6 >= 1000 then
					return false
				end
				local results_8
				if type(value_15) == "table" then
					results_8 = memo_6[Either_2]
					if results_8 == nil then
						results_8 = {}
						memo_6[Either_2] = results_8
					else
						local known_8 = results_8[value_15]
						if known_8 ~= nil then
							return known_8
						end
					end
				end
				depth_6 += 1
				local result_14 = Either_2(value_15)
				depth_6 -= 1
				if results_8 ~= nil then
					results_8[value_15] = result_14
				end
				return result_14
			end),
		}), t.interface({
			y = t.string,
			kids = t.array(function(value_16)
				if depth_6 >= 1000 then
					return false
				end
				local results_9
				if type(value_16) == "table" then
					results_9 = memo_6[Either_2]
					if results_9 == nil then
						results_9 = {}
						memo_6[Either_2] = results_9
					else
						local known_9 = results_9[value_16]
						if known_9 ~= nil then
							return known_9
						end
					end
				end
				depth_6 += 1
				local result_15 = Either_2(value_16)
				depth_6 -= 1
				if results_9 ~= nil then
					results_9[value_16] = result_15
				end
				return result_15
			end),
		}))
		return Either_2
	end)()
	return function(value_17)
		local outerDepth_6 = depth_6
		local outerMemo_6 = memo_6
		memo_6 = {}
		local ok_6, result_16 = pcall(Either_1, value_17)
		depth_6 = outerDepth_6
		memo_6 = outerMemo_6
		if not ok_6 then
			error(result_16, 0)
		end
		return result_16
	end
end)())
local aliasTree = Flamework.createGuard((function()
	local depth_7 = 0
	local memo_7
	local AliasTree_1
	AliasTree_1 = t.interface({
		value = t.number,
		kids = t.array(function(value_18)
			if depth_7 >= 1000 then
				return false
			end
			local results_10
			if type(value_18) == "table" then
				results_10 = memo_7[AliasTree_1]
				if results_10 == nil then
					results_10 = {}
					memo_7[AliasTree_1] = results_10
				else
					local known_10 = results_10[value_18]
					if known_10 ~= nil then
						return known_10
					end
				end
			end
			depth_7 += 1
			local result_17 = AliasTree_1(value_18)
			depth_7 -= 1
			if results_10 ~= nil then
				results_10[value_18] = result_17
			end
			return result_17
		end),
	})
	return function(value_19)
		local outerDepth_7 = depth_7
		local outerMemo_7 = memo_7
		memo_7 = {}
		local ok_7, result_18 = pcall(AliasTree_1, value_19)
		depth_7 = outerDepth_7
		memo_7 = outerMemo_7
		if not ok_7 then
			error(result_18, 0)
		end
		return result_18
	end
end)())
local forest = Flamework.createGuard((function()
	local depth_8 = 0
	local memo_8
	local TreeArray
	TreeArray = t.array(t.interface({
		value = t.number,
		kids = function(value_20)
			if depth_8 >= 1000 then
				return false
			end
			local results_11
			if type(value_20) == "table" then
				results_11 = memo_8[TreeArray]
				if results_11 == nil then
					results_11 = {}
					memo_8[TreeArray] = results_11
				else
					local known_11 = results_11[value_20]
					if known_11 ~= nil then
						return known_11
					end
				end
			end
			depth_8 += 1
			local result_19 = TreeArray(value_20)
			depth_8 -= 1
			if results_11 ~= nil then
				results_11[value_20] = result_19
			end
			return result_19
		end,
	}))
	return function(value_21)
		local outerDepth_8 = depth_8
		local outerMemo_8 = memo_8
		memo_8 = {}
		local ok_8, result_20 = pcall(TreeArray, value_21)
		depth_8 = outerDepth_8
		memo_8 = outerMemo_8
		if not ok_8 then
			error(result_20, 0)
		end
		return result_20
	end
end)())
local optionalTree = Flamework.createGuard((function()
	local depth_9 = 0
	local memo_9
	local Tree_2
	Tree_2 = t.interface({
		value = t.number,
		kids = t.array(function(value_22)
			if depth_9 >= 1000 then
				return false
			end
			local results_12
			if type(value_22) == "table" then
				results_12 = memo_9[Tree_2]
				if results_12 == nil then
					results_12 = {}
					memo_9[Tree_2] = results_12
				else
					local known_12 = results_12[value_22]
					if known_12 ~= nil then
						return known_12
					end
				end
			end
			depth_9 += 1
			local result_21 = Tree_2(value_22)
			depth_9 -= 1
			if results_12 ~= nil then
				results_12[value_22] = result_21
			end
			return result_21
		end),
	})
	local guard = t.optional(Tree_2)
	return function(value_23)
		local outerDepth_9 = depth_9
		local outerMemo_9 = memo_9
		memo_9 = {}
		local ok_9, result_22 = pcall(guard, value_23)
		depth_9 = outerDepth_9
		memo_9 = outerMemo_9
		if not ok_9 then
			error(result_22, 0)
		end
		return result_22
	end
end)())
local treeTuple = Flamework.createGuard((function()
	local depth_10 = 0
	local memo_10
	local Tree_3 = (function()
		local Tree_4
		Tree_4 = t.interface({
			value = t.number,
			kids = t.array(function(value_24)
				if depth_10 >= 1000 then
					return false
				end
				local results_13
				if type(value_24) == "table" then
					results_13 = memo_10[Tree_4]
					if results_13 == nil then
						results_13 = {}
						memo_10[Tree_4] = results_13
					else
						local known_13 = results_13[value_24]
						if known_13 ~= nil then
							return known_13
						end
					end
				end
				depth_10 += 1
				local result_23 = Tree_4(value_24)
				depth_10 -= 1
				if results_13 ~= nil then
					results_13[value_24] = result_23
				end
				return result_23
			end),
		})
		return Tree_4
	end)()
	local Chain_2
	Chain_2 = t.interface({
		value = t.number,
		next = t.optional(function(value_25)
			if depth_10 >= 1000 then
				return false
			end
			local results_14
			if type(value_25) == "table" then
				results_14 = memo_10[Chain_2]
				if results_14 == nil then
					results_14 = {}
					memo_10[Chain_2] = results_14
				else
					local known_14 = results_14[value_25]
					if known_14 ~= nil then
						return known_14
					end
				end
			end
			depth_10 += 1
			local result_24 = Chain_2(value_25)
			depth_10 -= 1
			if results_14 ~= nil then
				results_14[value_25] = result_24
			end
			return result_24
		end),
	})
	local guard_1 = (function(element, rest)
		return function(value_26)
			if not (type(value_26) == "table") then
				return false
			end
			local size = 0
			for key_1 in pairs(value_26) do
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
			local list = value_26
			if not element(list[1]) then
				return false
			end
			for index = 2, size do
				if not rest(list[index]) then
					return false
				end
			end
			return true
		end
	end)(Tree_3, Chain_2)
	return function(value_27)
		local outerDepth_10 = depth_10
		local outerMemo_10 = memo_10
		memo_10 = {}
		local ok_10, result_25 = pcall(guard_1, value_27)
		depth_10 = outerDepth_10
		memo_10 = outerMemo_10
		if not ok_10 then
			error(result_25, 0)
		end
		return result_25
	end
end)())
-- Past the dedup limit of 3: `Tree`'s guard is declared once, outside the guard that uses it.
local orchard = Flamework.createGuard((function()
	local depth_11 = 0
	local memo_11
	local Tree_5 = (function()
		local Tree_6
		Tree_6 = t.interface({
			value = t.number,
			kids = t.array(function(value_28)
				if depth_11 >= 1000 then
					return false
				end
				local results_15
				if type(value_28) == "table" then
					results_15 = memo_11[Tree_6]
					if results_15 == nil then
						results_15 = {}
						memo_11[Tree_6] = results_15
					else
						local known_15 = results_15[value_28]
						if known_15 ~= nil then
							return known_15
						end
					end
				end
				depth_11 += 1
				local result_26 = Tree_6(value_28)
				depth_11 -= 1
				if results_15 ~= nil then
					results_15[value_28] = result_26
				end
				return result_26
			end),
		})
		return Tree_6
	end)()
	local Chain_3
	Chain_3 = t.interface({
		value = t.number,
		next = t.optional(function(value_29)
			if depth_11 >= 1000 then
				return false
			end
			local results_16
			if type(value_29) == "table" then
				results_16 = memo_11[Chain_3]
				if results_16 == nil then
					results_16 = {}
					memo_11[Chain_3] = results_16
				else
					local known_16 = results_16[value_29]
					if known_16 ~= nil then
						return known_16
					end
				end
			end
			depth_11 += 1
			local result_27 = Chain_3(value_29)
			depth_11 -= 1
			if results_16 ~= nil then
				results_16[value_29] = result_27
			end
			return result_27
		end),
	})
	local guard_2 = t.interface({
		a = Tree_5,
		b = Tree_5,
		c = Tree_5,
		chain = Chain_3,
	})
	return function(value_30)
		local outerDepth_11 = depth_11
		local outerMemo_11 = memo_11
		memo_11 = {}
		local ok_11, result_28 = pcall(guard_2, value_30)
		depth_11 = outerDepth_11
		memo_11 = outerMemo_11
		if not ok_11 then
			error(result_28, 0)
		end
		return result_28
	end
end)())
return {
	tree = tree,
	chain = chain,
	menu = menu,
	json = json,
	directory = directory,
	node = node,
	either = either,
	aliasTree = aliasTree,
	forest = forest,
	optionalTree = optionalTree,
	treeTuple = treeTuple,
	orchard = orchard,
}
