// The global types a program needs without a lib (`noLib`).
interface Array<T> {
	length: number;
}
interface Boolean {}
interface CallableFunction {}
interface Function {}
interface IArguments {}
interface NewableFunction {}
interface Number {}
interface Object {}
interface RegExp {}
interface String {}
type Partial<T> = { [P in keyof T]?: T[P] };
