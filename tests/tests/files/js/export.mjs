export let letDecl;
export const constDecl = 1;
export function funDecl() {}
export class classDecl {}
let dummy = 0, local1 = 1, local2 = 2;
export { local1, local2 };
export { dummy as exportAs1 };
export { dummy as "export as string" };
export { dummy as default };

export * from "./mod11.mjs";
export * as exportNSAs from "./mod12.mjs";
export { exportedName1, exportedName2 } from "./mod13.mjs";
export { exportedName3 as exportAs2 } from "./mod14.mjs";
export { default as defaultAs } from "./mod15.mjs";
