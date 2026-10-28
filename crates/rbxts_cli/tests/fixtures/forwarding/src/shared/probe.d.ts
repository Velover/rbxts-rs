// The probe's calls these cases use: `@probe <op>` in the JSDoc.

/** @probe fail */
declare function probeFail(): void;

/** A warning at the call with the text of `text`, then the value. */
/** @probe diagsame */
declare function probeDiagSame<T>(value: T, text: string, asMessage?: boolean): T;
