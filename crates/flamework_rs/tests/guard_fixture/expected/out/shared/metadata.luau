-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local t = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@rbxts", "t", "lib", "ts").t
local Reflect = TS.import(script, game:GetService("ReplicatedStorage"), "rbxts_include", "node_modules", "@flamework-experimental", "core", "out").Reflect
-- Class metadata guards: `flamework:guard`, `flamework:parameter_guards` and `flamework:return_guard`.
-- @metadata reflect flamework:guard flamework:parameter_guards flamework:return_guard
local function Guarded()
	return function(target, key)
		print(target, key)
	end
end
-- @metadata reflect flamework:parameter_guards
local function Checked()
	return function(ctor)
		Reflect.defineMetadata(ctor, "checked", true)
	end
end
local Service
do
	Service = setmetatable({}, {
		__tostring = function()
			return "Service"
		end,
	})
	Service.__index = Service
	function Service.new(...)
		local self = setmetatable({}, Service)
		return self:constructor(...) or self
	end
	function Service:constructor(name, settings, count)
		self.name = name
		self.settings = settings
		self.count = count
		self.field = {}
	end
	function Service:method(a, b, ...)
		local rest = { ... }
		return b or {
			volume = a,
			tags = rest,
		}
	end
	function Service:walk(tree)
		return tree.kids
	end
	Service.promised = TS.async(function(self)
		return 1
	end)
	function Service:nothing()
	end
	do
		-- (Flamework) Service metadata
		Reflect.defineMetadata(Service, "flamework:parameter_guards", { t.string, t.interface({
			volume = t.number,
			muted = t.optional(t.boolean),
			tags = t.array(t.string),
		}), t.optional(t.number) })
		-- (Flamework) Service.field metadata
		Reflect.defineMetadata(Service, "flamework:guard", t.map(t.string, t.Vector3), "field")
		-- (Flamework) Service.optionalField metadata
		Reflect.defineMetadata(Service, "flamework:guard", t.optional((function(element, rest_1)
			return function(value_1)
				if not (type(value_1) == "table") then
					return false
				end
				local size = 0
				for key_1 in pairs(value_1) do
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
				local list = value_1
				if not element(list[1]) then
					return false
				end
				for index = 2, size do
					if not rest_1(list[index]) then
						return false
					end
				end
				return true
			end
		end)(t.number, t.string)), "optionalField")
		-- (Flamework) Service.tree metadata
		Reflect.defineMetadata(Service, "flamework:guard", (function()
			local depth = 0
			local memo
			local Tree_1
			Tree_1 = t.interface({
				value = t.number,
				kids = t.array(function(value_2)
					if depth >= 1000 then
						return false
					end
					local results
					if type(value_2) == "table" then
						results = memo[Tree_1]
						if results == nil then
							results = {}
							memo[Tree_1] = results
						else
							local known = results[value_2]
							if known ~= nil then
								return known
							end
						end
					end
					depth += 1
					local result = Tree_1(value_2)
					depth -= 1
					if results ~= nil then
						results[value_2] = result
					end
					return result
				end),
			})
			local guard = t.optional(Tree_1)
			return function(value_3)
				local outerDepth = depth
				local outerMemo = memo
				memo = {}
				local ok, result_1 = pcall(guard, value_3)
				depth = outerDepth
				memo = outerMemo
				if not ok then
					error(result_1, 0)
				end
				return result_1
			end
		end)(), "tree")
		-- (Flamework) Service.method metadata
		Reflect.defineMetadata(Service, "flamework:parameter_guards", { t.number, t.optional(t.interface({
			volume = t.number,
			muted = t.optional(t.boolean),
			tags = t.array(t.string),
		})), t.array(t.string) }, "method")
		Reflect.defineMetadata(Service, "flamework:return_guard", t.interface({
			volume = t.number,
			muted = t.optional(t.boolean),
			tags = t.array(t.string),
		}), "method")
		-- (Flamework) Service.walk metadata
		Reflect.defineMetadata(Service, "flamework:parameter_guards", { (function()
			local depth_1 = 0
			local memo_1
			local Tree_2
			Tree_2 = t.interface({
				value = t.number,
				kids = t.array(function(value_4)
					if depth_1 >= 1000 then
						return false
					end
					local results_1
					if type(value_4) == "table" then
						results_1 = memo_1[Tree_2]
						if results_1 == nil then
							results_1 = {}
							memo_1[Tree_2] = results_1
						else
							local known_1 = results_1[value_4]
							if known_1 ~= nil then
								return known_1
							end
						end
					end
					depth_1 += 1
					local result_2 = Tree_2(value_4)
					depth_1 -= 1
					if results_1 ~= nil then
						results_1[value_4] = result_2
					end
					return result_2
				end),
			})
			return function(value_5)
				local outerDepth_1 = depth_1
				local outerMemo_1 = memo_1
				memo_1 = {}
				local ok_1, result_3 = pcall(Tree_2, value_5)
				depth_1 = outerDepth_1
				memo_1 = outerMemo_1
				if not ok_1 then
					error(result_3, 0)
				end
				return result_3
			end
		end)() }, "walk")
		Reflect.defineMetadata(Service, "flamework:return_guard", (function()
			local depth_2 = 0
			local memo_2
			local TreeArray
			TreeArray = t.array(t.interface({
				value = t.number,
				kids = function(value_6)
					if depth_2 >= 1000 then
						return false
					end
					local results_2
					if type(value_6) == "table" then
						results_2 = memo_2[TreeArray]
						if results_2 == nil then
							results_2 = {}
							memo_2[TreeArray] = results_2
						else
							local known_2 = results_2[value_6]
							if known_2 ~= nil then
								return known_2
							end
						end
					end
					depth_2 += 1
					local result_4 = TreeArray(value_6)
					depth_2 -= 1
					if results_2 ~= nil then
						results_2[value_6] = result_4
					end
					return result_4
				end,
			}))
			return function(value_7)
				local outerDepth_2 = depth_2
				local outerMemo_2 = memo_2
				memo_2 = {}
				local ok_2, result_5 = pcall(TreeArray, value_7)
				depth_2 = outerDepth_2
				memo_2 = outerMemo_2
				if not ok_2 then
					error(result_5, 0)
				end
				return result_5
			end
		end)(), "walk")
		-- (Flamework) Service.promised metadata
		Reflect.defineMetadata(Service, "flamework:parameter_guards", {}, "promised")
		Reflect.defineMetadata(Service, "flamework:return_guard", TS.Promise.is, "promised")
		-- (Flamework) Service.nothing metadata
		Reflect.defineMetadata(Service, "flamework:parameter_guards", {}, "nothing")
		Reflect.defineMetadata(Service, "flamework:return_guard", t.none, "nothing")
	end
	Guarded()(Service, "field")
	Guarded()(Service, "optionalField")
	Guarded()(Service, "tree")
	local _descriptor = Guarded()(Service, "method", {
		value = Service.method,
	})
	if _descriptor then
		Service.method = _descriptor.value
	end
	local _descriptor_1 = Guarded()(Service, "walk", {
		value = Service.walk,
	})
	if _descriptor_1 then
		Service.walk = _descriptor_1.value
	end
	local _descriptor_2 = Guarded()(Service, "promised", {
		value = Service.promised,
	})
	if _descriptor_2 then
		Service.promised = _descriptor_2.value
	end
	local _descriptor_3 = Guarded()(Service, "nothing", {
		value = Service.nothing,
	})
	if _descriptor_3 then
		Service.nothing = _descriptor_3.value
	end
	Service = Checked()(Service) or Service
end
local Empty
do
	Empty = setmetatable({}, {
		__tostring = function()
			return "Empty"
		end,
	})
	Empty.__index = Empty
	function Empty.new(...)
		local self = setmetatable({}, Empty)
		return self:constructor(...) or self
	end
	function Empty:constructor()
	end
	do
		-- (Flamework) Empty metadata
		Reflect.defineMetadata(Empty, "flamework:parameter_guards", {})
	end
	Empty = Checked()(Empty) or Empty
end
local Holder
do
	Holder = setmetatable({}, {
		__tostring = function()
			return "Holder"
		end,
	})
	Holder.__index = Holder
	function Holder.new(...)
		local self = setmetatable({}, Holder)
		return self:constructor(...) or self
	end
	function Holder:constructor()
	end
	function Holder:run(item)
		return item
	end
	do
		-- (Flamework) Holder.value metadata
		Reflect.defineMetadata(Holder, "flamework:guard", t.string, "value")
		-- (Flamework) Holder.run metadata
		Reflect.defineMetadata(Holder, "flamework:parameter_guards", { t.string }, "run")
		Reflect.defineMetadata(Holder, "flamework:return_guard", t.string, "run")
	end
	Guarded()(Holder, "value")
	local _descriptor = Guarded()(Holder, "run", {
		value = Holder.run,
	})
	if _descriptor then
		Holder.run = _descriptor.value
	end
end
return {
	Guarded = Guarded,
	Checked = Checked,
	Service = Service,
	Empty = Empty,
	Holder = Holder,
}
