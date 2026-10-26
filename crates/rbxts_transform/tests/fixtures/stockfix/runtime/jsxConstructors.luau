return {
	ambient = {
		Vector2 = {
			new = function(x, y)
				return { X = x, Y = y }
			end,
		},
		UDim2 = {
			new = function(xs, xo, ys, yo)
				return { xs, xo, ys, yo }
			end,
			fromScale = function(x, y)
				return { x, 0, y, 0 }
			end,
		},
		Instance = {
			new = function(className)
				return { ClassName = className }
			end,
		},
	},
	calls = {
		{
			call = "pureChild",
			expect = function(element)
				local child = element.children[1]
				return element.props.Size == 1 and child.X == 1 and child.Y == 2
			end,
		},
		{ call = "userCallChild", expect = { { Size = 1 }, 2 } },
		{
			call = "instanceChild",
			expect = function(element)
				return element.props.Size == 1 and element.children[1].ClassName == "Part"
			end,
		},
		{
			call = "pureAttributes",
			expect = function(element)
				local p = element.props
				return p.Size[2] == 1 and p.Size[4] == 2 and p.Position[1] == 0.5 and element.children[1].X == 0
			end,
		},
		{ call = "shadowed", expect = { { Size = 1 }, 3 } },
	},
}
