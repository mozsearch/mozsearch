/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

// DO NOT copy this file over to the Firefox-vendored version of the plugin, it has its own Registration.cpp.

#include "clang/Frontend/FrontendPluginRegistry.h"

#include "MozsearchAction.h"

using namespace clang;

static FrontendPluginRegistry::Add<MozsearchAction>
    X("mozsearch-index", "create the mozsearch index database");
