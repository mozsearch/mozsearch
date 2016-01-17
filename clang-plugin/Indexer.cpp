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

// Needed for sha1 hacks
#include <fcntl.h>
#include <unistd.h>
#include "sha1.h"

using namespace clang;

const std::string GENERATED("__GENERATED__/");

std::string srcdir;
std::string objdir;
std::string outdir;

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

std::string
ToString(int n)
{
  char s[32];
  sprintf(s, "%06d", n);
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
  if (mangled.find("_external") == std::string::npos) {
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
GetMangledName(clang::MangleContext* ctx,
               const clang::NamedDecl* decl)
{
  llvm::SmallVector<char, 512> output;
  llvm::raw_svector_ostream out(output);
  ctx->mangleName(decl, out);
  return XPCOMHack(out.str().str());
}

#if 0
static std::string
GetMangledName(clang::MangleContext* ctx,
               const clang::Type* type)
{
  llvm::SmallVector<char, 512> output;
  llvm::raw_svector_ostream out(output);
  ctx->mangleTypeName(QualType(type, 0), out);
  return XPCOMHack(out.str().str());
}
#endif

// BEWARE: use only as a temporary
const char *
hash(std::string &str)
{
  static unsigned char rawhash[20];
  static char hashstr[41];
  sha1::calc(str.c_str(), str.size(), rawhash);
  sha1::toHexString(rawhash, hashstr);
  return hashstr;
}

std::string
EscapeString(std::string input)
{
  std::string output = "\"";
  char hex[] = { '0', '1', '2', '3', '4', '5', '6', '7',
                 '8', '9', 'a', 'b', 'c', 'd', 'e', 'f' };
  for (char c : input) {
    if (isspace(c) || c == '"' || c == '\\') {
      output += "\\x";
      output += hex[c >> 4];
      output += hex[c & 0xf];
    } else {
      output += c;
    }
  }
  output += '"';
  return output;
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

struct Comparator {
  bool operator()(std::string s1, std::string s2) {
    return s1 <= s2;
  }
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

  std::string LocationToString(SourceLocation loc) {
    std::string buffer;
    bool isInvalid;
    unsigned column = sm.getSpellingColumnNumber(loc, &isInvalid);

    if (!isInvalid) {
      unsigned line = sm.getSpellingLineNumber(loc, &isInvalid);
      if (!isInvalid) {
        buffer = ToString(line);
        buffer += ":";
        buffer += ToString(column - 1);  // Make 0-based.
      }
    }
    return buffer;
  }

public:
  IndexConsumer(CompilerInstance &ci)
   : ci(ci)
   , sm(ci.getSourceManager())
   , mMangleContext(nullptr)
  {
    //ci.getDiagnostics().setClient(this, false);
    //ci.getPreprocessor().addPPCallbacks(new PreprocThunk(this));
  }

  virtual DiagnosticConsumer *clone(DiagnosticsEngine &Diags) const {
    return new IndexConsumer(ci);
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

      // Okay, I want to use the standard library for I/O as much as possible,
      // but the C/C++ standard library does not have the feature of "open
      // succeeds only if it doesn't exist."
      FILE* fp = fopen(filename.c_str(), "w");
      if (fp == nullptr) {
        continue;
      }

      // There seems to be a bug where the comparator is called with the last
      // (invalid) iterator. Add a valid element there so we don't crash.
      info.output.push_back(std::string(""));
      stable_sort(info.output.begin(), info.output.end() - 1, Comparator());

      //write(fd, it->second->realname.c_str(), it->second->realname.length());
      //write(fd, "\n", 1);
      for (std::string& line : info.output) {
        fwrite(line.c_str(), line.length(), 1, fp);
      }
      fclose(fp);
    }
  }

  const CXXMethodDecl* FindRootMethod(const CXXMethodDecl* method) {
    CXXMethodDecl::method_iterator iter = method->begin_overridden_methods();
    CXXMethodDecl::method_iterator end = method->end_overridden_methods();
    for (; iter != end; iter++) {
      return FindRootMethod(*iter);
    }
    return method;
  }

  void VisitToken(const char *kind, SourceLocation loc, std::string mangled) {
    loc = sm.getSpellingLoc(loc);

    unsigned startOffset = sm.getFileOffset(loc);
    unsigned endOffset = startOffset + Lexer::MeasureTokenLength(loc, sm, ci.getLangOpts());

    std::string locStr = LocationToString(loc);

    const char* startChars = sm.getCharacterData(loc);
    std::string text(startChars, endOffset - startOffset);

    StringRef filename = sm.getFilename(loc);
    FileInfo *f = GetFileInfo(filename);

    char s[1024];
    sprintf(s, "%s %s %s %s\n", locStr.c_str(), kind, EscapeString(text).c_str(), mangled.c_str());

    f->output.push_back(std::string(s));
  }

  bool VisitFunctionDecl(FunctionDecl *d) {
    if (!IsInterestingLocation(d->getLocation())) {
      return true;
    }

    std::string mangled = GetMangledName(mMangleContext, d);
    if (CXXMethodDecl::classof(d)) {
      const CXXMethodDecl *method = FindRootMethod(dyn_cast<CXXMethodDecl>(d));
      mangled = GetMangledName(mMangleContext, method);
    }

    // FIXME: Need to skip the '~' token for destructors.
    SourceLocation loc = d->getLocation();

    const char* kind = d->isThisDeclarationADefinition() ? "def" : "decl";
    VisitToken(kind, loc, mangled);

    return true;
  }

#if 0
  bool VisitTagDecl(TagDecl *d) {
    if (!IsInterestingLocation(d->getLocation())) {
      return true;
    }

    SourceLocation loc = d->getLocation();
    std::string locStr = LocationToString(loc);
    printf("TAG %s\n", locStr.c_str());

    std::string mangled = GetMangledName(mMangleContext, d->getTypeForDecl());
    const char* kind = d->isThisDeclarationADefinition() ? "def" : "decl";
    VisitToken(kind, loc, mangled);

    return true;
  }
#endif

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
    VisitToken("use", loc, mangled);

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
      if (CXXMethodDecl::classof(namedCallee)) {
        namedCallee = FindRootMethod(dyn_cast<CXXMethodDecl>(namedCallee));
      }

      if (FunctionDecl::classof(namedCallee)) {
        const FunctionDecl *f = dyn_cast<FunctionDecl>(namedCallee);
        if (f->isTemplateInstantiation()) {
          namedCallee = f->getTemplateInstantiationPattern();
        }
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
        DeclRefExpr* ref = dyn_cast<DeclRefExpr>(callee);
        if (ref->hasQualifier()) {
          loc = ref->getNameInfo().getLoc();
        }
      }

      if (!loc.isValid()) {
        loc = callee->getLocStart();
        if (callee->getLocEnd() != loc) {
          // Skip this call. If we can't find a single token, we don't have a
          // good UI for displaying the call.
          return true;
        }
      }

      VisitToken("use", loc, mangled);
    }

    return true;
  }

#if 0
  bool VisitTagTypeLoc(TagTypeLoc tagLoc) {
    if (!IsInterestingLocation(tagLoc.getBeginLoc())) {
      return true;
    }

    TagDecl* decl = tagLoc.getDecl();
    std::string mangled = GetMangledName(mMangleContext, decl->getTypeForDecl());

    VisitToken("use", tagLoc.getBeginLoc(), mangled);

    return true;
  }
#endif
};

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
