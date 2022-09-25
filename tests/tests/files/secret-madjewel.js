// This file is secretly a mod-ule but with a name that is intended to defeat
// any filename heuristics we might introduce.

import { moduleConst, moduleFunc, moduleClass } from "./imported-module.mjs";
import { default as aliasedDefault } from "./imported-module.mjs";
import * as importedModule from "./imported-module.mjs";

const secretMadjewelConst = 11;
