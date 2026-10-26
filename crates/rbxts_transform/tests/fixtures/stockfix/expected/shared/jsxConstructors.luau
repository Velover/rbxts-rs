-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("include"):WaitForChild("RuntimeLib"))
-- The capture of `jsxQuirks`' `attributesBeforeChildren` is left out when the later prerequisites can't change what
-- was read first: a Roblox datatype constructor (`new Vector2(…)`, `UDim2.fromScale(…)`) only allocates. A user's
-- call or `Instance.new` in the same place still captures. (The fixture's tsconfig sets `preserveSymlinks`, so the
-- linked `@rbxts/types` counts as `@rbxts/types` and its globals are recognised.)
local React = TS.import(script, game:GetService("ReplicatedStorage"), "shared", "react")
local box = {
	size = 1,
}
-- `unset ?? x` checks `unset` against nil in a prerequisite (`false` is a value), so the children have prerequisites
local unset = nil
local function resize()
	box.size = 2
	return Vector2.new(0, 0)
end
-- the children's `Vector2.new` can't change `box.size`: the table is built in place
local function pureChild()
	box.size = 1
	local _condition = unset
	if _condition == nil then
		_condition = Vector2.new(1, 2)
	end
	return React.createElement("frame", {
		Size = box.size,
	}, _condition)
end
-- a user's call can: the table is captured before it, with the size from before the call
local function userCallChild()
	box.size = 1
	local _attributes = {
		Size = box.size,
	}
	local _condition = unset
	if _condition == nil then
		_condition = resize()
	end
	local element = React.createElement("frame", _attributes, _condition)
	return { element.props, box.size }
end
-- `Instance.new` is not on the list (a new instance can run code): captured as well
local function instanceChild()
	box.size = 1
	local _attributes = {
		Size = box.size,
	}
	local _condition = unset
	if _condition == nil then
		_condition = Instance.new("Part")
	end
	return React.createElement("frame", _attributes, _condition)
end
-- constructors in the attributes: nothing the children's call does can change them, so no capture
local function pureAttributes()
	local _condition = unset
	if _condition == nil then
		_condition = resize()
	end
	return React.createElement("frame", {
		Size = UDim2.new(0, 1, 0, 2),
		Position = UDim2.fromScale(0.5, 0.5),
	}, _condition)
end
-- a local named like a datatype is not the datatype: `UDim2.fromScale` here is the user's function
local function shadowed()
	local UDim2 = {
		fromScale = function(x, y)
			box.size = 3
			return x + y
		end,
	}
	box.size = 1
	local _attributes = {
		Size = box.size,
	}
	local _condition = unset
	if _condition == nil then
		_condition = UDim2.fromScale(1, 2)
	end
	local element = React.createElement("frame", _attributes, _condition)
	return { element.props, box.size }
end
return {
	pureChild = pureChild,
	userCallChild = userCallChild,
	instanceChild = instanceChild,
	pureAttributes = pureAttributes,
	shadowed = shadowed,
}
