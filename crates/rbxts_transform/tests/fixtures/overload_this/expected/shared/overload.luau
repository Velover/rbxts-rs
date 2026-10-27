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
return {
	over = over,
	first = first,
	byArg = byArg,
	withArrow = withArrow,
}
