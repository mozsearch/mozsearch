/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

#ifndef MozsearchAction_h_
#define MozsearchAction_h_

#include "clang/AST/AST.h"
#include "clang/AST/ASTConsumer.h"

class MozsearchAction : public clang::PluginASTAction {
public:
  std::unique_ptr<clang::ASTConsumer> CreateASTConsumer(clang::CompilerInstance &CI,
                                                 clang::StringRef F) override;

  bool ParseArgs(const clang::CompilerInstance &CI,
                 const std::vector<std::string> &Args) override;

  ActionType getActionType() override;
};

#endif /* MozsearchAction_h_ */
