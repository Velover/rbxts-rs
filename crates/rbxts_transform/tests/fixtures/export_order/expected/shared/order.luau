-- Compiled with roblox-ts v3.0.0
-- Export order (tests/fixtures.rs, `export_order`): TypeScript 5 binds a block's function declarations first, so the
-- export table lists the exported functions first, then the other exports in source order; TypeScript 7's API sorts
-- them in source order.
local first = 1
local Klass
do
	Klass = setmetatable({}, {
		__tostring = function()
			return "Klass"
		end,
	})
	Klass.__index = Klass
	function Klass.new(...)
		local self = setmetatable({}, Klass)
		return self:constructor(...) or self
	end
	function Klass:constructor()
	end
end
local function fn1()
	return first
end
local Kind
do
	local _inverse = {}
	Kind = setmetatable({}, {
		__index = _inverse,
	})
	Kind.A = 0
	_inverse[0] = "A"
end
local d1, d2 = 1, 2
local function fallback()
	return 0
end
local function fn2()
	return 2
end
local Space = {}
do
	local _container = Space
	local inner = 1
	_container.inner = inner
	local InnerClass
	do
		InnerClass = setmetatable({}, {
			__tostring = function()
				return "InnerClass"
			end,
		})
		InnerClass.__index = InnerClass
		function InnerClass.new(...)
			local self = setmetatable({}, InnerClass)
			return self:constructor(...) or self
		end
		function InnerClass:constructor()
		end
	end
	_container.InnerClass = InnerClass
	local function innerFn()
		return inner
	end
	_container.innerFn = innerFn
end
local function hidden()
	return 3
end
return {
	fn1 = fn1,
	default = fallback,
	fn2 = fn2,
	first = first,
	Klass = Klass,
	aliased = first,
	Kind = Kind,
	d1 = d1,
	d2 = d2,
	Space = Space,
	hidden = hidden,
}
