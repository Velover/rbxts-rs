-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("include"):WaitForChild("RuntimeLib"))
-- `continue` inside a switch continues the loop around it. roblox-ts lowers a switch to a `repeat … until true`
-- block and wrote `continue` as it was: that continued the block, leaving the switch like `break`, so the rest of the
-- loop body still ran.
local function forOf()
	local log = {}
	for _, x in { 1, 2, 3 } do
		local _continue = false
		repeat
			if x == 2 then
				_continue = true
				break
			end
			table.insert(log, `case {x}`)
		until true
		if _continue then
			continue
		end
		table.insert(log, `after {x}`)
	end
	return log
end
-- a `default` before other cases (the other lowering)
local function middleDefault()
	local log = {}
	for _, x in { 1, 2, 3 } do
		local _continue = false
		repeat
			if x == 1 then
				table.insert(log, "one")
				break
			end
			local _case
			if x == 3 then
				_case = 1
			end
			if _case == nil then
				_continue = true
				break
			end
			if _case == 1 then
				table.insert(log, "three")
			end
		until true
		if _continue then
			continue
		end
		table.insert(log, `after {x}`)
	end
	return log
end
-- a switch inside a switch: the inner `continue` leaves both
local function nested()
	local log = {}
	local i = 0
	while i < 4 do
		i += 1
		local _exp = i % 2
		local _continue = false
		repeat
			if _exp == 0 then
				local _continue_1 = false
				repeat
					if i == 2 then
						_continue_1 = true
						break
					end
				until true
				if _continue_1 then
					_continue = true
					break
				end
				table.insert(log, `even {i}`)
				break
			end
			table.insert(log, `odd {i}`)
		until true
		if _continue then
			continue
		end
		table.insert(log, `after {i}`)
	end
	return log
end
-- a loop inside a switch: its `continue` is that loop's, as it was
local function loopInSwitch()
	local log = {}
	for _, x in { 1, 2 } do
		local _continue = false
		repeat
			if x == 1 then
				for _1, y in { 1, 2, 3 } do
					if y == 2 then
						continue
					end
					table.insert(log, `y {y}`)
				end
				break
			end
			_continue = true
			break
		until true
		if _continue then
			continue
		end
		table.insert(log, `after {x}`)
	end
	return log
end
-- a `for` loop's increment still runs, and a `do … while` loop's condition is still checked
local function forAndDo()
	local log = {}
	for i = 0, 3 do
		local _exp = i % 2
		local _continue = false
		repeat
			if _exp == 0 then
				_continue = true
				break
			end
		until true
		if _continue then
			continue
		end
		table.insert(log, `for {i}`)
	end
	local j = 0
	repeat
		do
			j += 1
			local _continue = false
			repeat
				if j == 1 then
					_continue = true
					break
				end
			until true
			if _continue then
				continue
			end
			table.insert(log, `do {j}`)
		end
	until not (j < 3)
	return log
end
-- a `continue` inside a try statement inside a switch (its function returns, and the switch passes it on)
local function tryInSwitch(values)
	local log = {}
	for _, x in values do
		local _continue = false
		repeat
			local _fallthrough = false
			if x == 2 then
				local _exitType, _returns = TS.try(function()
					return TS.TRY_CONTINUE
				end, nil, function()
					table.insert(log, `finally {x}`)
				end)
				if _exitType then
					_continue = true
					break
				end
			end
		until true
		if _continue then
			continue
		end
		table.insert(log, `after {x}`)
	end
	return log
end
return {
	forOf = forOf,
	middleDefault = middleDefault,
	nested = nested,
	loopInSwitch = loopInSwitch,
	forAndDo = forAndDo,
	tryInSwitch = tryInSwitch,
}
