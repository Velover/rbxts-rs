// a call inside a parenthesized decorator expression is an ordinary call (the decorator's expression is not a call)
@(probeDecoInject("parenthesized"))
export class P1 {}

@probeDecoInject("direct")
export class P2 {}

export class P3 {
	@(probeDecoInject("method"))
	method() {}
}
