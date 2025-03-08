export const mozsrc1 = 11;

import { mozsrc2 } from "./mozsrc2.mjs";
export { mozsrc3 } from "./mozsrc3.mjs";
const { sub } = await import("./subdir/sub.mjs");

const ns2 = await import("./non-existent.mjs");
