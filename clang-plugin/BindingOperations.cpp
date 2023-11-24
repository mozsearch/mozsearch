/* -*- Mode: C++; tab-width: 2; indent-tabs-mode: nil; c-basic-offset: 2 -*- */
/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

#include "BindingOperations.h"

#include <clang/AST/Attr.h>
#include <clang/AST/Expr.h>

#include <algorithm>
#include <array>
#include <string>
#include <vector>

#ifdef __cpp_lib_optional
#include <optional>
template<typename T> using optional = std::optional<T>;
#else
#include <llvm/ADT/Optional.h>
template<typename T> using optional = clang::Optional<T>;
#endif

using namespace clang;

namespace {

struct AbstractBinding {
  // Subset of tools/analysis/BindingSlotLang
  enum class Lang {
    Cpp,
    Jvm,
  };
  static constexpr size_t LangLength = 2;
  static constexpr std::array<StringRef, LangLength> langNames = {
    "cpp",
    "jvm",
  };

  static optional<Lang> langFromString(StringRef langName)
  {
    const auto it = std::find(langNames.begin(), langNames.end(), langName);
    if (it == langNames.end())
      return {};

    return Lang(it - langNames.begin());
  }
  static StringRef stringFromLang(Lang lang)
  {
    return langNames[size_t(lang)];
  }

  // Subset of tools/analysis/BindingSlotKind
  enum class Kind {
    Class,
    Method,
    Getter,
    Setter,
    Const,
  };
  static constexpr size_t KindLength = 5;
  static constexpr std::array<StringRef, KindLength> kindNames = {
    "class",
    "method",
    "getter",
    "setter",
    "const",
  };

  static optional<Kind> kindFromString(StringRef kindName)
  {
    const auto it = std::find(kindNames.begin(), kindNames.end(), kindName);
    if (it == kindNames.end())
      return {};

    return Kind(it - kindNames.begin());
  }
  static StringRef stringFromKind(Kind kind)
  {
    return kindNames[size_t(kind)];
  }

  Lang lang;
  Kind kind;
  StringRef symbol;
};
constexpr size_t AbstractBinding::KindLength;
constexpr std::array<StringRef, AbstractBinding::KindLength> AbstractBinding::kindNames;
constexpr size_t AbstractBinding::LangLength;
constexpr std::array<StringRef, AbstractBinding::LangLength> AbstractBinding::langNames;

struct BindingTo : public AbstractBinding {
  BindingTo(AbstractBinding b) : AbstractBinding(std::move(b)) {}
  static constexpr StringRef ANNOTATION = "binding_to";
};
constexpr StringRef BindingTo::ANNOTATION;

struct BoundAs : public AbstractBinding {
  BoundAs(AbstractBinding b) : AbstractBinding(std::move(b)) {}
  static constexpr StringRef ANNOTATION = "bound_as";
};
constexpr StringRef BoundAs::ANNOTATION;

template<typename B>
void setBindingAttr(ASTContext &C, Decl &decl, B binding)
{
  // recent LLVM: CreateImplicit then setDelayedArgs
  Expr *langExpr = StringLiteral::Create(C, AbstractBinding::stringFromLang(binding.lang), StringLiteral::UTF8, false, {}, {});
  Expr *kindExpr = StringLiteral::Create(C, AbstractBinding::stringFromKind(binding.kind), StringLiteral::UTF8, false, {}, {});
  Expr *symbolExpr = StringLiteral::Create(C, binding.symbol, StringLiteral::UTF8, false, {}, {});
  auto **args = new (C, 16) Expr *[3]{langExpr, kindExpr, symbolExpr};
  auto *attr = AnnotateAttr::CreateImplicit(C, B::ANNOTATION, args, 3);
  decl.addAttr(attr);
}

optional<AbstractBinding> readBinding(const AnnotateAttr &attr)
{
  if (attr.args_size() != 3)
    return {};

  const auto *langExpr = attr.args().begin()[0];
  const auto *kindExpr = attr.args().begin()[1];
  const auto *symbolExpr = attr.args().begin()[2];
  if (!langExpr || !kindExpr || !symbolExpr)
    return {};

  const auto *langName = dyn_cast<StringLiteral>(langExpr->IgnoreUnlessSpelledInSource());
  const auto *kindName = dyn_cast<StringLiteral>(kindExpr->IgnoreUnlessSpelledInSource());
  const auto *symbol = dyn_cast<StringLiteral>(symbolExpr->IgnoreUnlessSpelledInSource());
  if (!langName || !kindName || !symbol)
    return {};

  const auto lang = AbstractBinding::langFromString(langName->getString());
  const auto kind = AbstractBinding::kindFromString(kindName->getString());

  if (!lang || !kind)
    return {};

  return AbstractBinding {
    .lang = *lang,
    .kind = *kind,
    .symbol = symbol->getString(),
  };
}

optional<BindingTo> getBindingTo(const Decl &decl)
{
  for (const auto *attr : decl.specific_attrs<AnnotateAttr>()) {
    if (attr->getAnnotation() != BindingTo::ANNOTATION)
      continue;

    const auto binding = readBinding(*attr);
    if (!binding)
      continue;

    return BindingTo{*binding};
  }
  return {};
}

// C++23: turn into generator
std::vector<BoundAs> getBoundAs(const Decl &decl)
{
  std::vector<BoundAs> found;

  for (const auto *attr : decl.specific_attrs<AnnotateAttr>()) {
    if (attr->getAnnotation() != BoundAs::ANNOTATION)
      continue;

    const auto binding = readBinding(*attr);
    if (!binding)
      continue;

    found.push_back(BoundAs{*binding});
  }

  return found;
}

void addSlotOwnerAttribute(llvm::json::OStream &J, const Decl &decl)
{
  if (const auto bindingTo = getBindingTo(decl)) {
    J.attributeBegin("slotOwner");
    J.objectBegin();
    J.attribute("slotKind", AbstractBinding::stringFromKind(bindingTo->kind));
    J.attribute("slotLang", "cpp");
    J.attribute("ownerLang", AbstractBinding::stringFromLang(bindingTo->lang));
    J.attribute("sym", bindingTo->symbol);
    J.objectEnd();
    J.attributeEnd();
  }
}
void addBindingSlotsAttribute(llvm::json::OStream &J, const Decl &decl)
{
  const auto allBoundAs = getBoundAs(decl);
  if (!allBoundAs.empty()) {
    J.attributeBegin("bindingSlots");
    J.arrayBegin();
    for (const auto boundAs : allBoundAs) {
      J.objectBegin();
      J.attribute("slotKind", AbstractBinding::stringFromKind(boundAs.kind));
      J.attribute("slotLang", AbstractBinding::stringFromLang(boundAs.lang));
      J.attribute("ownerLang", "cpp");
      J.attribute("sym", boundAs.symbol);
      J.objectEnd();
    }
    J.arrayEnd();
    J.attributeEnd();
  }
}

} // anonymous namespace

void emitBindingAttributes(llvm::json::OStream &J, const Decl &decl)
{
  addSlotOwnerAttribute(J, decl);
  addBindingSlotsAttribute(J, decl);
}
