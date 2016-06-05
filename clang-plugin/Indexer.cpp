/* -*- Mode: C++; tab-width: 2; indent-tabs-mode: nil; c-basic-offset: 2 -*- */

#include "clang/AST/AST.h"
#include "clang/AST/ASTConsumer.h"
#include "clang/AST/Expr.h"
#include "clang/AST/ExprCXX.h"
#include "clang/AST/RecursiveASTVisitor.h"
#include "clang/Basic/SourceManager.h"
#include "clang/Basic/Version.h"
#include "clang/Frontend/CompilerInstance.h"
#include "clang/Frontend/FrontendPluginRegistry.h"
#include "clang/Lex/Lexer.h"
#include "clang/Lex/Preprocessor.h"
#include "clang/Lex/PPCallbacks.h"
#include "clang/AST/Mangle.h"
#include "llvm/ADT/SmallString.h"
#include "llvm/Support/raw_ostream.h"

#include <memory>
#include <iostream>
#include <map>
#include <sstream>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/file.h>

// Needed for sha1 hacks
#include <fcntl.h>
#include <unistd.h>
#include "sha1.h"

using namespace clang;

const std::string GENERATED("__GENERATED__/");

std::string srcdir;
std::string objdir;
std::string outdir;

static std::string
Hash(const std::string& str)
{
  static unsigned char rawhash[20];
  static char hashstr[41];
  sha1::calc(str.c_str(), str.size(), rawhash);
  sha1::toHexString(rawhash, hashstr);
  return std::string(hashstr);
}

void
EnsurePath(std::string path)
{
  size_t pos = 0;
  if (path[0] == '/') {
    pos++;
  }

  while ((pos = path.find('/', pos)) != std::string::npos) {
    std::string portion = path.substr(0, pos);
    int err = mkdir(portion.c_str(), 0775);
    if (err == -1 && errno != EEXIST) {
      perror("mkdir failed");
      exit(1);
    }

    pos++;
  }
}

static std::string
ToString(int n)
{
  char s[32];
  sprintf(s, "%d", n);
  return std::string(s);
}

std::string
ReplaceAll(std::string mangled, std::string pattern, std::string replacement)
{
  size_t pos = 0;
  while ((pos = mangled.find(pattern, pos)) != std::string::npos) {
    mangled = mangled.replace(pos, pattern.length(), replacement);
    pos += replacement.length();
  }
  return mangled;
}

std::string
XPCOMHack(std::string mangled)
{
  if (mangled.find("_external") == std::string::npos &&
      mangled.find("_internal") == std::string::npos) {
    return mangled;
  }

  const char* replacements[][2] = {
    {"nsString",                       "nsString_external"},
    {"nsCString",                      "nsCString_external"},
    {"nsDependentString",              "nsDependentString_external"},
    {"nsDependentCString",             "nsDependentCString_external"},
    {"NS_ConvertASCIItoUTF16",         "NS_ConvertASCIItoUTF16_external"},
    {"NS_ConvertUTF8toUTF16",          "NS_ConvertUTF8toUTF16_external"},
    {"NS_ConvertUTF16toUTF8",          "NS_ConvertUTF16toUTF8_external"},
    {"NS_LossyConvertUTF16toASCII",    "NS_LossyConvertUTF16toASCII_external"},
    {"nsGetterCopies",                 "nsGetterCopies_external"},
    {"nsCGetterCopies",                "nsCGetterCopies_external"},
    {"nsDependentSubstring",           "nsDependentSubstring_external"},
    {"nsDependentCSubstring",          "nsDependentCSubstring_external"},
    {"nsAString",                      "nsAString_internal"},
    {"nsACString",                     "nsACString_internal"},
  };
  size_t length = sizeof(replacements) / sizeof(*replacements);

  for (size_t i = 0; i < length; i++) {
    std::string pattern = replacements[i][1];
    pattern = ToString(pattern.length()) + pattern;

    std::string replacement = replacements[i][0];
    replacement = ToString(replacement.length()) + replacement;

    mangled = ReplaceAll(mangled, pattern, replacement);
  }
  return mangled;
}

static std::string
EscapeString(std::string input)
{
  std::string output = "";
  char hex[] = { '0', '1', '2', '3', '4', '5', '6', '7',
                 '8', '9', 'a', 'b', 'c', 'd', 'e', 'f' };
  for (char c : input) {
    if (isspace(c) || c == '"' || c == '\\') {
      output += "\\u00";
      output += hex[c >> 4];
      output += hex[c & 0xf];
    } else {
      output += c;
    }
  }
  return output;
}

static bool
IsValidToken(std::string input)
{
  for (char c : input) {
    if (isspace(c) || c == '"' || c == '\\') {
      return false;
    }
  }
  return true;
}

class IndexConsumer;

struct FileInfo
{
  FileInfo(std::string &rname) : realname(rname) {
    if (rname.compare(0, objdir.length(), objdir) == 0) {
      // We're in the objdir, so we are probably a generated header
      // We use the escape character to indicate the objdir nature.
      // Note that output also has the `/' already placed
      interesting = true;
      realname.replace(0, objdir.length(), GENERATED);
      return;
    }

    interesting = rname.compare(0, srcdir.length(), srcdir) == 0;
    if (interesting) {
      // Remove the trailing `/' as well.
      realname.erase(0, srcdir.length() + 1);
    }
  }
  std::string realname;
  std::vector<std::string> output;
  bool interesting;
};

class IndexConsumer;

class PreprocessorHook : public PPCallbacks
{
  IndexConsumer* indexer;

public:
  PreprocessorHook(IndexConsumer *c) : indexer(c) {}

  virtual void MacroDefined(const Token &tok, const MacroDirective *md);

  virtual void MacroExpands(const Token &tok, const MacroDirective *md,
                            SourceRange range, const MacroArgs *ma);
  virtual void MacroUndefined(const Token &tok, const MacroDirective *md);
  virtual void Defined(const Token &tok, const MacroDirective *md, SourceRange range);
  virtual void Ifdef(SourceLocation loc, const Token &tok, const MacroDirective *md);
  virtual void Ifndef(SourceLocation loc, const Token &tok, const MacroDirective *md);

#if 0
  virtual void InclusionDirective(SourceLocation hashLoc,
                                  const Token &includeTok,
                                  StringRef fileName,
                                  bool isAngled,
                                  CharSourceRange filenameRange,
                                  const FileEntry *file,
                                  StringRef searchPath,
                                  StringRef relativePath,
                                  const Module *imported);
#endif
};

class IndexConsumer : public ASTConsumer,
                      public RecursiveASTVisitor<IndexConsumer>,
                      public DiagnosticConsumer
{
private:
  CompilerInstance &ci;
  SourceManager &sm;
  std::ostream *out;
  std::map<std::string, FileInfo *> relmap;
  MangleContext *mMangleContext;

  FileInfo *GetFileInfo(const std::string &filename) {
    std::map<std::string, FileInfo *>::iterator it;
    it = relmap.find(filename);
    if (it == relmap.end()) {
      // We haven't seen this file before. We need to make the FileInfo
      // structure information ourselves
      const char *real = realpath(filename.c_str(), NULL);
      std::string realstr(real ? real : filename.c_str());
      it = relmap.find(realstr);
      if (it == relmap.end()) {
        // Still didn't find it. Make the FileInfo structure
        FileInfo *info = new FileInfo(realstr);
        it = relmap.insert(make_pair(realstr, info)).first;
      }
      it = relmap.insert(make_pair(filename, it->second)).first;
    }
    return it->second;
  }

  FileInfo *GetFileInfo(const char *filename) {
    std::string filenamestr(filename);
    return GetFileInfo(filenamestr);
  }

  // Helpers for processing declarations
  // Should we ignore this location?
  bool IsInterestingLocation(SourceLocation loc) {
    // If we don't have a valid location... it's probably not interesting.
    if (loc.isInvalid())
      return false;
    // I'm not sure this is the best, since it's affected by #line and #file
    // et al. On the other hand, if I just do spelling, I get really wrong
    // values for locations in macros, especially when ## is involved.
    // TODO: So yeah, maybe use sm.getFilename(loc) instead.
    std::string filename = sm.getPresumedLoc(loc).getFilename();
    // Invalid locations and built-ins: not interesting at all
    if (filename[0] == '<')
      return false;

    // Get the real filename
    FileInfo *f = GetFileInfo(filename);
    return f->interesting;
  }

  std::string LocationToString(SourceLocation loc, size_t length = 0) {
    std::string buffer;
    bool isInvalid;
    unsigned column = sm.getSpellingColumnNumber(loc, &isInvalid);

    if (!isInvalid) {
      unsigned line = sm.getSpellingLineNumber(loc, &isInvalid);
      if (!isInvalid) {
        buffer = ToString(line);
        buffer += ":";
        buffer += ToString(column - 1);  // Make 0-based.
        if (length) {
          buffer += "-";
          buffer += ToString(column - 1 + length);
        }
      }
    }
    return buffer;
  }

  std::string MangleLocation(SourceLocation loc) {
    FileInfo *f = GetFileInfo(sm.getPresumedLoc(loc).getFilename());
    if (f) {
      std::string filename = f->realname;
      return Hash(filename + std::string("@") + LocationToString(loc));
    } else {
      return std::string("?");
    }
  }

  std::string MangleQualifiedName(std::string name) {
    std::replace(name.begin(), name.end(), ' ', '_');
    return name;
  }

  std::string GetMangledName(clang::MangleContext* ctx,
                             const clang::NamedDecl* decl) {
    if (isa<FunctionDecl>(decl) || isa<VarDecl>(decl)) {
      if (const FunctionDecl* f = dyn_cast<FunctionDecl>(decl)) {
        if (f->isTemplateInstantiation()) {
          *(int *)0 = 0;
        }
      }

      const DeclContext* dc = decl->getDeclContext();
      if (isa<TranslationUnitDecl>(dc) ||
          isa<NamespaceDecl>(dc) ||
          isa<LinkageSpecDecl>(dc) ||
          //isa<ExternCContextDecl>(dc) ||
          isa<TagDecl>(dc))
        {
          llvm::SmallVector<char, 512> output;
          llvm::raw_svector_ostream out(output);
          if (const CXXConstructorDecl* d = dyn_cast<CXXConstructorDecl>(decl)) {
            ctx->mangleCXXCtor(d, CXXCtorType::Ctor_Complete, out);
          } else if (const CXXDestructorDecl* d = dyn_cast<CXXDestructorDecl>(decl)) {
            ctx->mangleCXXDtor(d, CXXDtorType::Dtor_Complete, out);
          } else {
            ctx->mangleName(decl, out);
          }
          return XPCOMHack(out.str().str());
        } else {
          return std::string("V_") + MangleLocation(decl->getLocation()) + std::string("_") +
            Hash(decl->getName());
        }
    } else if (isa<TagDecl>(decl) || isa<TypedefNameDecl>(decl)) {
      if (!decl->getIdentifier()) {
        // Anonymous.
        return std::string("T_") + MangleLocation(decl->getLocation());
      }

      return std::string("T_") + MangleQualifiedName(decl->getQualifiedNameAsString());
    } else if (isa<NamespaceDecl>(decl) || isa<NamespaceAliasDecl>(decl)) {
      if (!decl->getIdentifier()) {
        // Anonymous.
        return std::string("NS_") + MangleLocation(decl->getLocation());
      }

      return std::string("NS_") + MangleQualifiedName(decl->getQualifiedNameAsString());
    } else if (const FieldDecl* d2 = dyn_cast<FieldDecl>(decl)) {
      const RecordDecl* record = d2->getParent();
      return std::string("F_<") + GetMangledName(ctx, record) + ">_" + ToString(d2->getFieldIndex());
    } else if (const EnumConstantDecl* d2 = dyn_cast<EnumConstantDecl>(decl)) {
      const DeclContext* dc = decl->getDeclContext();
      if (const NamedDecl* named = dyn_cast<NamedDecl>(dc)) {
        return std::string("E_<") + GetMangledName(ctx, named) + ">_" + d2->getNameAsString();
      }
    }

    assert(false);
    return std::string("");
  }

  void DebugLocation(SourceLocation loc) {
    std::string s = LocationToString(loc);
    StringRef filename = sm.getFilename(loc);
    printf("--> %s %s\n", std::string(filename).c_str(), s.c_str());
  }

public:
  IndexConsumer(CompilerInstance &ci)
   : ci(ci)
   , sm(ci.getSourceManager())
   , mMangleContext(nullptr)
  {
    //ci.getDiagnostics().setClient(this, false);
    ci.getPreprocessor().addPPCallbacks(llvm::make_unique<PreprocessorHook>(this));
  }

  virtual DiagnosticConsumer *clone(DiagnosticsEngine &Diags) const {
    return new IndexConsumer(ci);
  }

  bool shouldVisitTemplateInstantiations() const {
    //return true;
    return false;
  }

  // All we need is to follow the final declaration.
  virtual void HandleTranslationUnit(ASTContext &ctx) {
    mMangleContext = clang::ItaniumMangleContext::create(ctx, ci.getDiagnostics());

    TraverseDecl(ctx.getTranslationUnitDecl());

    // Emit all files now
    std::map<std::string, FileInfo *>::iterator it;
    for (it = relmap.begin(); it != relmap.end(); it++) {
      if (!it->second->interesting)
        continue;

      FileInfo& info = *it->second;

      std::string filename = outdir;
      filename += it->second->realname;

      EnsurePath(filename);

      int fd = open(filename.c_str(), O_RDWR | O_CREAT, 0666);
      if (fd == -1) {
        continue;
      }

      do {
        int rv = flock(fd, LOCK_EX);
        if (rv == 0) {
          break;
        }
      } while (true);

      std::vector<std::string> lines;

      char buffer[65536];
      FILE* fp = fdopen(dup(fd), "r");
      while (fgets(buffer, sizeof(buffer), fp)) {
        lines.push_back(std::string(buffer));
      }
      fclose(fp);

      lines.insert(lines.end(), info.output.begin(), info.output.end());

      std::sort(lines.begin(), lines.end());

      std::vector<std::string> nodupes;

      std::unique_copy(lines.begin(), lines.end(), std::back_inserter(nodupes));

      lseek(fd, 0, SEEK_SET);

      fp = fdopen(fd, "w");
      size_t length = 0;
      for (std::string& line : nodupes) {
        length += line.length();
        fwrite(line.c_str(), line.length(), 1, fp);
      }
      ftruncate(fd, length);
      fclose(fp);
    }
  }

  void FindOverriddenMethods(const CXXMethodDecl* method, std::vector<std::string>& symbols) {
    std::string mangled = GetMangledName(mMangleContext, method);
    symbols.push_back(mangled);

    CXXMethodDecl::method_iterator iter = method->begin_overridden_methods();
    CXXMethodDecl::method_iterator end = method->end_overridden_methods();
    for (; iter != end; iter++) {
      const CXXMethodDecl* decl = *iter;
      if (decl->isTemplateInstantiation()) {
        decl = dyn_cast<CXXMethodDecl>(decl->getTemplateInstantiationPattern());
      }
      return FindOverriddenMethods(decl, symbols);
    }
  }

  enum {
    NO_CROSSREF = 1,
  };

  void VisitToken(const char *kind,
                  const char *prettyKind, const char *prettyData,
                  std::string targetPretty,
                  SourceLocation loc, const std::vector<std::string>& symbols,
                  int flags = 0)
  {
    loc = sm.getSpellingLoc(loc);

    unsigned startOffset = sm.getFileOffset(loc);
    unsigned endOffset = startOffset + Lexer::MeasureTokenLength(loc, sm, ci.getLangOpts());

    std::string locStr = LocationToString(loc);
    std::string locStr2 = LocationToString(loc, endOffset - startOffset);

    const char* startChars = sm.getCharacterData(loc);
    std::string text(startChars, endOffset - startOffset);

    StringRef filename = sm.getFilename(loc);
    FileInfo *f = GetFileInfo(filename);

    if (!IsValidToken(text)) {
      return;
    }

    size_t maxlen = 0;
    for (auto it = symbols.begin(); it != symbols.end(); it++) {
      maxlen = std::max(it->length(), maxlen);
    }

    std::string symbolList;
    {
      char *s = new char[1024 + targetPretty.length() + maxlen];
      for (auto it = symbols.begin(); it != symbols.end(); it++) {
        std::string symbol = *it;

        if (!(flags & NO_CROSSREF)) {
          sprintf(s, "{\"loc\":\"%s\", \"target\":1, \"kind\":\"%s\", \"pretty\": \"%s\", \"sym\":\"%s\"}\n",
                  locStr.c_str(), kind, targetPretty.c_str(), symbol.c_str());
          f->output.push_back(std::string(s));
        }

        if (it != symbols.begin()) {
          symbolList += ",";
        }
        symbolList += symbol;
      }
      delete[] s;
    }

    char* buf = new char[1024 + symbolList.length()];

    const char* no_crossref = "";
    if (flags & NO_CROSSREF) {
      no_crossref = ", \"no_crossref\":1";
    }

    sprintf(buf,
            "{\"loc\":\"%s\", \"source\":1, \"syntax\": \"%s,%s\", "
            "\"pretty\":\"%s %s\", \"sym\":\"%s\"%s}\n",
            locStr2.c_str(), kind, prettyKind,
            prettyKind, prettyData ? prettyData : EscapeString(text).c_str(),
            symbolList.c_str(),
            no_crossref);
    f->output.push_back(std::string(buf));

    delete[] buf;
  }

  void VisitToken(const char *kind,
                  const char *prettyKind, const char *prettyData,
                  std::string targetPretty,
                  SourceLocation loc, std::string symbol, int flags = 0)
  {
    std::vector<std::string> v = { symbol };
    VisitToken(kind, prettyKind, prettyData, targetPretty, loc, v, flags);
  }

  bool VisitNamedDecl(NamedDecl *d) {
    if (!IsInterestingLocation(d->getLocation())) {
      return true;
    }

    if (isa<ParmVarDecl>(d) && !d->getDeclName().getAsIdentifierInfo()) {
      // Unnamed parameter in function proto.
      return true;
    }

    int flags = 0;
    const char* kind = "def";
    const char* prettyKind = "?";
    if (FunctionDecl* d2 = dyn_cast<FunctionDecl>(d)) {
      if (d2->isTemplateInstantiation()) {
        d = d2->getTemplateInstantiationPattern();
      }
      kind = d2->isThisDeclarationADefinition() ? "def" : "decl";
      prettyKind = "function";
    } else if (TagDecl* d2 = dyn_cast<TagDecl>(d)) {
      kind = d2->isThisDeclarationADefinition() ? "def" : "decl";
      prettyKind = "type";
    } else if (isa<TypedefNameDecl>(d)) {
      kind = "def";
      prettyKind = "type";
    } else if (VarDecl* d2 = dyn_cast<VarDecl>(d)) {
      if (d2->isLocalVarDeclOrParm()) {
        flags = NO_CROSSREF;
      }

      kind = d2->isThisDeclarationADefinition() == VarDecl::DeclarationOnly ? "decl" : "def";
      prettyKind = "variable";
    } else if (isa<NamespaceDecl>(d) || isa<NamespaceAliasDecl>(d)) {
      kind = "def";
      prettyKind = "namespace";
    } else if (isa<FieldDecl>(d)) {
      kind = "def";
      prettyKind = "field";
    } else if (isa<EnumConstantDecl>(d)) {
      kind = "def";
      prettyKind = "enum constant";
    } else {
      return true;
    }

    std::vector<std::string> symbols = { GetMangledName(mMangleContext, d) };
    if (CXXMethodDecl::classof(d)) {
      symbols.clear();
      FindOverriddenMethods(dyn_cast<CXXMethodDecl>(d), symbols);
    }

    SourceLocation loc = d->getLocation();

    // For destructors, loc points to the ~ character. We want to skip to the
    // class name.
    if (isa<CXXDestructorDecl>(d)) {
      const char* p = sm.getCharacterData(loc);
      assert(p == '~');
      p++;

      unsigned skipped = 1;
      while (*p == ' ' || *p == '\t' || *p == '\r' || *p == '\n') {
        p++;
        skipped++;
      }

      loc = loc.getLocWithOffset(skipped);

      prettyKind = "destructor";
    }

    VisitToken(kind, prettyKind, nullptr, d->getQualifiedNameAsString(), loc, symbols, flags);

    return true;
  }

  bool VisitCXXConstructExpr(CXXConstructExpr* e) {
    if (!IsInterestingLocation(e->getLocStart())) {
      return true;
    }

    FunctionDecl* ctor = e->getConstructor();
    if (ctor->isTemplateInstantiation()) {
      ctor = ctor->getTemplateInstantiationPattern();
    }
    std::string mangled = GetMangledName(mMangleContext, ctor);

    // FIXME: Need to do something different for list initialization.

    SourceLocation loc = e->getLocStart();
    VisitToken("use", "constructor", ctor->getNameAsString().c_str(), ctor->getQualifiedNameAsString(), loc, mangled);

    return true;
  }

  bool VisitCallExpr(CallExpr *e) {
    if (!IsInterestingLocation(e->getLocStart())) {
      return true;
    }

    Decl *callee = e->getCalleeDecl();
    if (!callee ||
        !IsInterestingLocation(callee->getLocation()) ||
        !NamedDecl::classof(callee)) {
      return true;
    }

    const NamedDecl *namedCallee = dyn_cast<NamedDecl>(callee);

    if (namedCallee) {
      if (!FunctionDecl::classof(namedCallee)) {
        return true;
      }

      const FunctionDecl *f = dyn_cast<FunctionDecl>(namedCallee);
      if (f->isTemplateInstantiation()) {
        namedCallee = f->getTemplateInstantiationPattern();
      }

      std::string mangled = GetMangledName(mMangleContext, namedCallee);

      Expr* callee = e->getCallee()->IgnoreParenImpCasts();

      SourceLocation loc;
      if (CXXOperatorCallExpr::classof(e)) {
        // Just take the first token.
        CXXOperatorCallExpr* op = dyn_cast<CXXOperatorCallExpr>(e);
        loc = op->getOperatorLoc();
      } else if (MemberExpr::classof(callee)) {
        MemberExpr* member = dyn_cast<MemberExpr>(callee);
        loc = member->getMemberLoc();
      } else if (DeclRefExpr::classof(callee)) {
        // We handle this in VisitDeclRefExpr.
        return true;
      }

      if (!loc.isValid()) {
        loc = callee->getLocStart();
        if (callee->getLocEnd() != loc) {
          // Skip this call. If we can't find a single token, we don't have a
          // good UI for displaying the call.
          return true;
        }
      }

      VisitToken("use", "function", nullptr, namedCallee->getQualifiedNameAsString(), loc, mangled);
    }

    return true;
  }

  bool VisitTagTypeLoc(TagTypeLoc l) {
    if (!IsInterestingLocation(l.getBeginLoc())) {
      return true;
    }

    TagDecl* decl = l.getDecl();
    std::string mangled = GetMangledName(mMangleContext, decl);
    VisitToken("use", "type", nullptr, decl->getQualifiedNameAsString(), l.getBeginLoc(), mangled);
    return true;
  }

  bool VisitTypedefTypeLoc(TypedefTypeLoc l) {
    if (!IsInterestingLocation(l.getBeginLoc())) {
      return true;
    }

    NamedDecl* decl = l.getTypedefNameDecl();
    std::string mangled = GetMangledName(mMangleContext, decl);
    VisitToken("use", "type", nullptr, decl->getQualifiedNameAsString(), l.getBeginLoc(), mangled);
    return true;
  }

  bool VisitInjectedClassNameTypeLoc(InjectedClassNameTypeLoc l) {
    if (!IsInterestingLocation(l.getBeginLoc())) {
      return true;
    }

    NamedDecl* decl = l.getDecl();
    std::string mangled = GetMangledName(mMangleContext, decl);
    VisitToken("use", "type", nullptr, decl->getQualifiedNameAsString(), l.getBeginLoc(), mangled);
    return true;
  }

  bool VisitTemplateSpecializationTypeLoc(TemplateSpecializationTypeLoc l) {
    if (!IsInterestingLocation(l.getBeginLoc())) {
      return true;
    }

    TemplateDecl* td = l.getTypePtr()->getTemplateName().getAsTemplateDecl();
    if (ClassTemplateDecl *d = dyn_cast<ClassTemplateDecl>(td)) {
      NamedDecl* decl = d->getTemplatedDecl();
      std::string mangled = GetMangledName(mMangleContext, decl);
      VisitToken("use", "type", nullptr, decl->getQualifiedNameAsString(), l.getBeginLoc(), mangled);
    }

    return true;
  }

  bool VisitDeclRefExpr(DeclRefExpr *e) {
    if (!IsInterestingLocation(e->getExprLoc())) {
      return true;
    }

    SourceLocation loc = e->getExprLoc();
    if (e->hasQualifier()) {
      loc = e->getNameInfo().getLoc();
    }

    NamedDecl* decl = e->getDecl();
    if (const VarDecl* d2 = dyn_cast<VarDecl>(decl)) {
      int flags = 0;
      if (d2->isLocalVarDeclOrParm()) {
        flags = NO_CROSSREF;
      }
      std::string mangled = GetMangledName(mMangleContext, decl);
      VisitToken("use", "variable", nullptr, decl->getQualifiedNameAsString(), loc, mangled, flags);
    } else if (isa<FunctionDecl>(decl)) {
      const FunctionDecl *f = dyn_cast<FunctionDecl>(decl);
      if (f->isTemplateInstantiation()) {
        decl = f->getTemplateInstantiationPattern();
      }

      std::string mangled = GetMangledName(mMangleContext, decl);
      VisitToken("use", "function", nullptr, decl->getQualifiedNameAsString(), loc, mangled);
    } else if (isa<EnumConstantDecl>(decl)) {
      std::string mangled = GetMangledName(mMangleContext, decl);
      VisitToken("use", "enum", nullptr, decl->getQualifiedNameAsString(), loc, mangled);
    }

    return true;
  }

  bool VisitCXXConstructorDecl(CXXConstructorDecl *d) {
    if (!IsInterestingLocation(d->getLocation())) {
      return true;
    }

    for (CXXConstructorDecl::init_const_iterator it = d->init_begin(); it != d->init_end(); ++it) {
      const CXXCtorInitializer *ci = *it;
      if (!ci->getMember() || !ci->isWritten()) {
        continue;
      }
      FieldDecl* member = ci->getMember();
      std::string mangled = GetMangledName(mMangleContext, member);
      VisitToken("use", "field", nullptr, member->getQualifiedNameAsString(), ci->getMemberLocation(), mangled);
    }

    return true;
  }

  bool VisitMemberExpr(MemberExpr* e) {
    if (!IsInterestingLocation(e->getExprLoc())) {
      return true;
    }

    ValueDecl* decl = e->getMemberDecl();
    if (FieldDecl* field = dyn_cast<FieldDecl>(decl)) {
      std::string mangled = GetMangledName(mMangleContext, field);
      VisitToken("use", "field", nullptr, field->getQualifiedNameAsString(), e->getExprLoc(), mangled);
    }
    return true;
  }

  void MacroDefined(const Token &tok, const MacroDirective *macro) {
    if (macro->getMacroInfo()->isBuiltinMacro()) {
      return;
    }
    if (!IsInterestingLocation(tok.getLocation())) {
      return;
    }

    SourceLocation loc = tok.getLocation();
    IdentifierInfo* ident = tok.getIdentifierInfo();
    if (ident) {
      std::string mangled = std::string("M_") + MangleLocation(loc);
      VisitToken("def", "macro", nullptr, ident->getName(), loc, mangled);
    }
  }

  void MacroUsed(const Token &tok, const MacroInfo *macro) {
    if (macro->isBuiltinMacro()) {
      return;
    }
    if (!IsInterestingLocation(tok.getLocation())) {
      return;
    }

    SourceLocation loc = macro->getDefinitionLoc();
    IdentifierInfo* ident = tok.getIdentifierInfo();
    if (ident) {
      std::string mangled = std::string("M_") + MangleLocation(loc);
      VisitToken("use", "macro", nullptr, ident->getName(), tok.getLocation(), mangled);
    }
  }
};

void
PreprocessorHook::MacroDefined(const Token &tok, const MacroDirective *md)
{
  indexer->MacroDefined(tok, md);
}

void
PreprocessorHook::MacroExpands(const Token &tok, const MacroDirective *md,
                               SourceRange range, const MacroArgs *ma)
{
  indexer->MacroUsed(tok, md->getMacroInfo());
}

void
PreprocessorHook::MacroUndefined(const Token &tok, const MacroDirective *md)
{
  if (md) {
    indexer->MacroUsed(tok, md->getMacroInfo());
  }
}

void
PreprocessorHook::Defined(const Token &tok, const MacroDirective *md, SourceRange range)
{
  if (md) {
    indexer->MacroUsed(tok, md->getMacroInfo());
  }
}

void
PreprocessorHook::Ifdef(SourceLocation loc, const Token &tok, const MacroDirective *md)
{
  if (md) {
    indexer->MacroUsed(tok, md->getMacroInfo());
  }
}

void
PreprocessorHook::Ifndef(SourceLocation loc, const Token &tok, const MacroDirective *md)
{
  if (md) {
    indexer->MacroUsed(tok, md->getMacroInfo());
  }
}

class IndexAction : public PluginASTAction
{
protected:
  std::unique_ptr<ASTConsumer> CreateASTConsumer(CompilerInstance &CI, llvm::StringRef f) {
    return llvm::make_unique<IndexConsumer>(CI);
  }

  bool ParseArgs(const CompilerInstance &CI,
                 const std::vector<std::string>& args)
  {
    if (args.size() != 3) {
      DiagnosticsEngine &D = CI.getDiagnostics();
      unsigned DiagID = D.getCustomDiagID(DiagnosticsEngine::Error,
        "Need arguments for the source, output, and object directories");
      D.Report(DiagID);
      return false;
    }
    // Load our directories
    char *abs_src = realpath(args[0].c_str(), NULL);
    if (!abs_src) {
      DiagnosticsEngine &D = CI.getDiagnostics();
      unsigned DiagID = D.getCustomDiagID(DiagnosticsEngine::Error,
        "Source directory '%0' does not exist");
      D.Report(DiagID) << args[0];
      return false;
    }
    srcdir = abs_src;

    char *abs_outdir = realpath(args[1].c_str(), NULL);
    if (!abs_outdir) {
      DiagnosticsEngine &D = CI.getDiagnostics();
      unsigned DiagID = D.getCustomDiagID(DiagnosticsEngine::Error,
        "Output directory '%0' does not exist");
      D.Report(DiagID) << args[1];
      return false;
    }
    outdir = abs_outdir;
    outdir += "/";

    char *abs_objdir = realpath(args[2].c_str(), NULL);
    if (!abs_objdir) {
      DiagnosticsEngine &D = CI.getDiagnostics();
      unsigned DiagID = D.getCustomDiagID(DiagnosticsEngine::Error,
        "Objdir '%0' does not exist");
      D.Report(DiagID) << args[2];
      return false;
    }
    objdir = abs_objdir;
    objdir += "/";

    return true;
  }

  void PrintHelp(llvm::raw_ostream& ros) {
    ros << "Help for mozsearch plugin goes here\n";
  }
};

static FrontendPluginRegistry::Add<IndexAction>
X("mozsearch-index", "create the mozsearch index database");
