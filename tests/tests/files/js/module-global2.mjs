// See also module-global1.mjs.


const ModuleGlobalTest_global_conflict = 20;

import { ModuleGlobalTest_exported_and_imported } from "./module-global1.mjs";
const ModuleGlobalTest_exported_and_global = 20;
export { ModuleGlobalTest_exported_and_reexported } from "./module-global1.mjs";

[

  ModuleGlobalTest_global_conflict,

  ModuleGlobalTest_exported_and_imported,
  ModuleGlobalTest_exported_and_global,
  ModuleGlobalTest_exported_and_reexported,
];
