/* -*- Mode: C++; tab-width: 2; indent-tabs-mode: nil; c-basic-offset: 2 -*- */

#include "clang/AST/AST.h"
#include "clang/AST/ASTConsumer.h"
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

#include <iostream>
#include <map>
#include <sstream>
#include <stdio.h>
#include <stdlib.h>

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
GetMangledName(clang::MangleContext* ctx,
               const clang::NamedDecl* decl)
{
  llvm::SmallVector<char, 512> output;
  llvm::raw_svector_ostream out(output);
  ctx->mangleName(decl, out);
  return out.str().str();
}

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

class IndexConsumer;

struct FileInfo
{
  FileInfo(std::string &rname) : realname(rname) {
    interesting = rname.compare(0, srcdir.length(), srcdir) == 0;
    if (interesting) {
      // Remove the trailing `/' as well.
      realname.erase(0, srcdir.length() + 1);
    } else if (rname.compare(0, objdir.length(), objdir) == 0) {
      // We're in the objdir, so we are probably a generated header
      // We use the escape character to indicate the objdir nature.
      // Note that obj also has the `/' already placed
      interesting = true;
      realname.replace(0, objdir.length(), GENERATED);
    }
  }
  std::string realname;
  std::ostringstream output;
  bool interesting;
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
    // Since we're dealing with only expansion locations here, we should be
    // guaranteed to stay within the same file as "out" points to.
    unsigned column = sm.getExpansionColumnNumber(loc, &isInvalid);

    if (!isInvalid) {
      unsigned line = sm.getExpansionLineNumber(loc, &isInvalid);
      if (!isInvalid) {
        buffer = std::to_string(line);
        buffer += ":";
        buffer += std::to_string(column - 1);  // Make 0-based.
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
      // Look at how much code we have
      std::string content = it->second->output.str();
      if (content.length() == 0)
        continue;
      std::string filename = outdir;
      filename += it->second->realname;

      // Okay, I want to use the standard library for I/O as much as possible,
      // but the C/C++ standard library does not have the feature of "open
      // succeeds only if it doesn't exist."
      int fd = open(filename.c_str(), O_WRONLY | O_CREAT | O_TRUNC, 0644);
      if (fd != -1) {
        write(fd, content.c_str(), content.length());
        close(fd);
      }
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

  bool VisitFunctionDecl(FunctionDecl *d) {
    if (!IsInterestingLocation(d->getLocation())) {
      return true;
    }

    std::string mangled = GetMangledName(mMangleContext, d);
    if (CXXMethodDecl::classof(d)) {
      const CXXMethodDecl *method = FindRootMethod(dyn_cast<CXXMethodDecl>(d));
      mangled = GetMangledName(mMangleContext, method);
    }

    std::string loc = LocationToString(d->getLocation());

    StringRef filename = sm.getFilename(d->getLocation());
    FileInfo *f = GetFileInfo(filename);
    f->output << loc << " def " << d->getNameAsString() << " " << mangled << "\n";
    //printf("%s def %s %s\n", loc.c_str(), d->getNameAsString().c_str(), mangled.c_str());

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
      std::string mangled = GetMangledName(mMangleContext, namedCallee);
      if (CXXMethodDecl::classof(namedCallee)) {
        const CXXMethodDecl *method = FindRootMethod(dyn_cast<CXXMethodDecl>(namedCallee));
        mangled = GetMangledName(mMangleContext, method);
      }

      if (sm.isMacroBodyExpansion(e->getCallee()->getLocStart())) {
        return true;
      }

      SourceLocation start = sm.getExpansionLoc(e->getCallee()->getLocStart());
      SourceLocation end = sm.getExpansionRange(e->getCallee()->getLocEnd()).second;

      unsigned startOffset = sm.getFileOffset(start);
      unsigned endOffset = sm.getFileOffset(end);

      endOffset += Lexer::MeasureTokenLength(end, sm, ci.getLangOpts());

      std::string loc = LocationToString(start);

      const char* startChars = sm.getCharacterData(start);
      std::string text(startChars, endOffset - startOffset);

      StringRef filename = sm.getFilename(start);
      FileInfo *f = GetFileInfo(filename);
      f->output << loc << " use " << text << " " << mangled << "\n";
      //printf("%s use %s %s\n", loc.c_str(), text.c_str(), mangled.c_str());
    }

    return true;
  }

};

class IndexAction : public PluginASTAction
{
protected:
  ASTConsumer *CreateASTConsumer(CompilerInstance &CI, llvm::StringRef f) {
    return new IndexConsumer(CI);
  }

  bool ParseArgs(const CompilerInstance &CI,
                 const std::vector<std::string>& args)
  {
    if (args.size() != 1) {
      DiagnosticsEngine &D = CI.getDiagnostics();
      unsigned DiagID = D.getCustomDiagID(DiagnosticsEngine::Error,
        "Need an argument for the source directory");
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

    const char *env = getenv("MOZSEARCH_OBJDIR");
    if (env) {
      objdir = env;
    } else {
      objdir = srcdir;
    }
    char *abs_objdir = realpath(objdir.c_str(), NULL);
    if (!abs_objdir) {
      DiagnosticsEngine &D = CI.getDiagnostics();
      unsigned DiagID = D.getCustomDiagID(DiagnosticsEngine::Error,
        "Objdir '%0' does not exist");
      D.Report(DiagID) << objdir;
      return false;
    }
    objdir = realpath(objdir.c_str(), NULL);
    objdir += "/";

    env = getenv("MOZSEARCH_OUTDIR");
    assert(env);
    if (env) {
      outdir = env;
    } else {
      outdir = srcdir;
    }
    char* abs_outdir = realpath(outdir.c_str(), NULL);
    if (!abs_outdir) {
      DiagnosticsEngine &D = CI.getDiagnostics();
      unsigned DiagID = D.getCustomDiagID(DiagnosticsEngine::Error,
        "Output directory '%0' does not exist");
      D.Report(DiagID) << outdir;
      return false;
    }
    outdir = realpath(outdir.c_str(), NULL);
    outdir += "/";

    return true;
  }

  void PrintHelp(llvm::raw_ostream& ros) {
    ros << "Help for mozsearch plugin goes here\n";
  }
};

static FrontendPluginRegistry::Add<IndexAction>
X("mozsearch-index", "create the mozsearch index database");
