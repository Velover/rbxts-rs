import { Flamework } from "@flamework-experimental/core";

// Template literal and string mapping types.
export const suffix = Flamework.createGuard<`${string}-id`>();
export const prefixNumber = Flamework.createGuard<`item_${number}`>();
export const special = Flamework.createGuard<`a.b(${string})%[x]*+?^$`>();
export const unionTemplate = Flamework.createGuard<`${"a" | "b"}-${string}`>();
export const lower = Flamework.createGuard<Lowercase<string>>();
export const templateOrNumber = Flamework.createGuard<`#${string}` | number>();
