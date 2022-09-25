import { moduleConst, moduleFunc, ModuleClass } from "./imported-module.mjs";
import { default as aliasedDefault } from "./imported-module.mjs";
import * as importedModule from "./imported-module.mjs";

function exportedAsDefaultAsReference() {
    return 5;
}

// This should generate a failure.
export default exportedAsDefaultAsReference;

const rootModuleConst = 10;
