return {
	calls = {
		{ call = "forOf", expect = { "case 1", "after 1", "case 3", "after 3" } },
		{ call = "middleDefault", expect = { "one", "after 1", "three", "after 3" } },
		{ call = "nested", expect = { "odd 1", "after 1", "odd 3", "after 3", "even 4", "after 4" } },
		{ call = "loopInSwitch", expect = { "y 1", "y 3", "after 1" } },
		{ call = "forAndDo", expect = { "for 1", "for 3", "do 2", "do 3" } },
		{ call = "tryInSwitch", args = { { 1, 2, 3 } }, label = "({1, 2, 3})", expect = { "after 1", "finally 2", "after 3" } },
	},
}
