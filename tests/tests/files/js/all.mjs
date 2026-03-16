// ModuleDeclaration
// ??

// ModuleRequest
// --

// ImportAttribute

// ImportDeclaration
// ImportSpecifier
// ImportNamespaceSpecifier
import { NAME_IMPORT_1 } from "MOD";
import { NAME_IMPORT_2 as DEF_IMPORT_1 } from "MOD";
import DEF_IMPORT_DEFAULT_1 from "MOD";
import * as DEF_IMPORT_DEFAULT_2 from "MOD";
import { default as DEF_IMPORT_DEFAULT_3 } from "MOD";
import { "STR" as DEF_IMPORT_STR } from "MOD";
import DEF_IMPORT_DEFAULT_4, { NAME_IMPORT_3 } from "MOD";
import "MOD";

// ImportSourceDeclaration
// --

// ExportDeclaration
// ExportSpecifier
// ExportNamespaceSpecifier
// ExportBatchSpecifier
export let DEF_EXPORT_1 = USE_EXPORT_1;
export const DEF_EXPORT_2 = USE_EXPORT_2;
export function DEF_EXPORT_FUNC(NAME_EXPORT_1 = USE_EXPORT_3) {
  return NAME_EXPORT_2;
}
export function* DEF_EXPORT_GEN(NAME_EXPORT_3 = USE_EXPORT_4) {
  return NAME_EXPORT_4;
}
export class DEF_EXPORT_CLASS {
  constructor(NAME_EXPORT_5) {
    USE_EXPORT_5;
  }
};
var DEF_VAR_1 = 0, DEF_VAR_2 = 0;
var DEF_VAR_3 = 0, DEF_VAR_4 = 0;
var DEF_VAR_5 = 0;
export { DEF_VAR_1, DEF_VAR_2 };
export { DEF_VAR_3 as NAME_EXPORT_6 };
export { DEF_VAR_4 as "STR" };
export { DEF_VAR_5 as default };

export * from "MOD";
export * as NAME_EXPORT_7 from "MOD";
export { NAME_EXPORT_8 } from "MOD";
export { NAME_EXPORT_9 as NAME_EXPORT_10 } from "MOD";
export { default as NAME_EXPORT_11 } from "MOD";
