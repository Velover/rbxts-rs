-- Compiled with roblox-ts v3.0.0
-- One user macro per marker kind; the call sites are in the other files.
-- @metadata macro
local function idOf(id)
	return id
end
-- @metadata macro
local function textOf(text)
	return text
end
-- @metadata macro
local function dependencyOf(info)
	return info
end
-- @metadata macro
local function conciseOf(info)
	return info
end
-- @metadata macro
local function where(line, character, width, uuid, text)
	return { line, character, width, uuid, text }
end
-- @metadata macro
local function callsite(uuid)
	return uuid
end
-- @metadata macro
local function obfuscated(name)
	return name
end
-- @metadata macro
local function labelsOf(labels)
	return labels
end
-- @metadata macro
local function emit(value)
	return value
end
-- @metadata macro
local function constant(value)
	return value
end
-- @metadata macro
local function pathOf(_path, path)
	return path
end
-- @metadata macro
local function globOf(_glob, glob)
	return glob
end
-- @metadata macro
local function keysOf(value)
	return value
end
-- @metadata macro
local function shuffled(value)
	return value
end
-- @metadata macro
local function fieldInfo(meta)
	return meta
end
-- @metadata macro
local function describe(meta)
	return meta
end
-- @metadata macro
local function memberCount(meta)
	return meta
end
-- @metadata macro
local function transformerVersion(meta)
	return meta
end
-- A parameter that takes metadata or a value: the first marked member of the union generates.
-- @metadata macro
local function either(id)
	return id
end
-- Padding: the generated parameter is the second one.
-- @metadata macro
local function second(first, id)
	return `{first}/{id}`
end
-- Inlined: the call is replaced by its generated value.
-- @metadata macro {@link text intrinsic-inline}
-- A literal argument is required.
-- @metadata macro {@link options intrinsic-const}
local function withOptions(options, id)
	return { options, id }
end
-- A macro method and a macro class.
local Registry
do
	Registry = setmetatable({}, {
		__tostring = function()
			return "Registry"
		end,
	})
	Registry.__index = Registry
	function Registry.new(...)
		local self = setmetatable({}, Registry)
		return self:constructor(...) or self
	end
	function Registry:constructor()
		self.entries = {}
	end
	function Registry:add(value, id)
		self.entries[id] = value
		return self
	end
end
-- @metadata macro
local Tagged
do
	Tagged = setmetatable({}, {
		__tostring = function()
			return "Tagged"
		end,
	})
	Tagged.__index = Tagged
	function Tagged.new(...)
		local self = setmetatable({}, Tagged)
		return self:constructor(...) or self
	end
	function Tagged:constructor(id)
		self.id = id
	end
end
-- A decorator macro.
-- @metadata macro
local function Mark(id)
	return function(target)
		print(target, id)
	end
end
return {
	idOf = idOf,
	textOf = textOf,
	dependencyOf = dependencyOf,
	conciseOf = conciseOf,
	where = where,
	callsite = callsite,
	obfuscated = obfuscated,
	labelsOf = labelsOf,
	emit = emit,
	constant = constant,
	pathOf = pathOf,
	globOf = globOf,
	keysOf = keysOf,
	shuffled = shuffled,
	fieldInfo = fieldInfo,
	describe = describe,
	memberCount = memberCount,
	transformerVersion = transformerVersion,
	either = either,
	second = second,
	withOptions = withOptions,
	Mark = Mark,
	Registry = Registry,
	Tagged = Tagged,
}
