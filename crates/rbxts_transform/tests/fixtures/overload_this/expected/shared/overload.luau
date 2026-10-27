-- Compiled with roblox-ts v3.0.0
-- Overloads whose candidates disagree on `this`. TypeScript 5 gives a function expression argument the `this` of the
-- first overload it checks the function against in full, even when that overload is then rejected.
-- the first overload is checked and rejected on the return type; the function keeps its `this: void`
local function over(cb)
end
over(function() end)
over(function() end)
over((function() end))
-- the first overload takes it: `this: Foo`
local function first(cb)
end
first(function(self) end)
-- another argument rules the first overload out before the function is checked: the second one's `this`
local function byArg(x, cb)
end
byArg(1, function(self) end)
byArg("a", function() end)
-- an arrow function argument is left out too (context sensitive in both), the function after it is the one decided
local function withArrow(a, cb)
end
withArrow(function(x) end, function() end)
-- a method call: the overloads of a method
holder:take(function() end)
-- an arrow argument before the function passes the overload's first pass but not its full check: the function is
-- checked against the second overload in full
local function arrowFirst(a, cb)
end
arrowFirst(function(x)
	return x
end, function(self) end)
-- generic overloads
local function gen(v, cb)
end
gen(1, function() end)
-- a spread argument
local function spr(cb, ...)
	local rest = { ... }
end
local nums = { 1, 2 }
spr(function() end, unpack(nums))
-- explicit type arguments
local function targ(cb, v)
end
targ(function() end)
-- an optional chain
local _result = maybe
if _result ~= nil then
	_result:take(function() end)
end
-- an object literal method in another argument
local function objArg(o, cb)
end
objArg({
	m = function(self) end,
}, function() end)
-- `new` with construct overloads
local made = Ctor.new(function() end)
return {
	over = over,
	first = first,
	byArg = byArg,
	withArrow = withArrow,
	arrowFirst = arrowFirst,
	gen = gen,
	spr = spr,
	targ = targ,
	objArg = objArg,
	made = made,
}
