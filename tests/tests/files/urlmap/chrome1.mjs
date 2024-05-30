export const chrome1 = 11;

import { chrome2 } from "./chrome2.mjs";
export { chrome3 } from "./chrome3.mjs";
const { sub } = await import("./subdir/sub.mjs");

const ns2 = await import("./non-existent.mjs");
