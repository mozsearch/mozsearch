// See also module-global2.mjs.

const ModuleGlobalTest_global_unique = 10;
const ModuleGlobalTest_global_conflict = 10;
export const ModuleGlobalTest_exported = 10;
export const ModuleGlobalTest_exported_and_imported = 10;
export const ModuleGlobalTest_exported_and_global = 10;
export const ModuleGlobalTest_exported_and_reexported = 10;

[
  ModuleGlobalTest_global_unique,
  ModuleGlobalTest_global_conflict,
  ModuleGlobalTest_exported,
  ModuleGlobalTest_exported_and_imported,
  ModuleGlobalTest_exported_and_global,
  ModuleGlobalTest_exported_and_reexported,
];
