/* -*- Mode: C++; tab-width: 2; indent-tabs-mode: nil; c-basic-offset: 2 -*- */

#include "clang/AST/AST.h"
#include "clang/AST/ASTConsumer.h"
#include "clang/AST/ASTContext.h"
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
#include <unordered_set>
#include <sstream>
#include <tuple>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <sys/time.h>
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

template<typename ... Args>
std::string
StringFormat(const std::string& format, Args... args)
{
  size_t len = snprintf(nullptr, 0, format.c_str(), args...);
  std::unique_ptr<char[]> buf(new char[len + 1]);
  snprintf(buf.get(), len + 1, format.c_str(), args...);
  return std::string(buf.get(), buf.get() + len);
}

static std::string
Hash(const std::string& str)
{
  static unsigned char rawhash[20];
  static char hashstr[41];
  sha1::calc(str.c_str(), str.size(), rawhash);
  sha1::toHexString(rawhash, hashstr);
  return std::string(hashstr);
}

static double
Time()
{
  struct timeval tv;
  gettimeofday(&tv, nullptr);
  return double(tv.tv_sec) + double(tv.tv_usec) / 1000000.;
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
  return StringFormat("%d", n);
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

  virtual void MacroDefined(const Token &tok, const MacroDirective *md) override;

  virtual void MacroExpands(const Token &tok, const MacroDefinition& md,
                            SourceRange range, const MacroArgs *ma) override;
  virtual void MacroUndefined(const Token &tok, const MacroDefinition& md) override;
  virtual void Defined(const Token &tok, const MacroDefinition& md, SourceRange range) override;
  virtual void Ifdef(SourceLocation loc, const Token &tok, const MacroDefinition& md) override;
  virtual void Ifndef(SourceLocation loc, const Token &tok, const MacroDefinition& md) override;

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

class JSONFormatter
{
  struct Property {
    const char* mName;
    const char* mLiteralValue;
    const std::string* mStringValue;
    int mIntValue;
  };

  static const int kMaxProperties = 32;

  Property mProperties[kMaxProperties];
  int mPropertyCount;
  size_t mLength;

 public:
  JSONFormatter()
   : mPropertyCount(0)
   , mLength(0)
  {}

  void Add(const char* name, const char* value) {
    assert(mPropertyCount < kMaxProperties);
    mProperties[mPropertyCount].mName = name;
    mProperties[mPropertyCount].mLiteralValue = value;
    mProperties[mPropertyCount].mStringValue = nullptr;
    mPropertyCount++;

    mLength += strlen(name) + 3 + strlen(value) + 2 + 1;
  }

  void Add(const char* name, const std::string& value) {
    assert(mPropertyCount < kMaxProperties);
    mProperties[mPropertyCount].mName = name;
    mProperties[mPropertyCount].mLiteralValue = nullptr;
    mProperties[mPropertyCount].mStringValue = &value;
    mPropertyCount++;

    mLength += strlen(name) + 3 + value.length() + 2 + 1;
  }

  void Add(const char* name, int value) {
    // 1 digit
    assert(value >= 0 && value < 10);

    assert(mPropertyCount < kMaxProperties);
    mProperties[mPropertyCount].mName = name;
    mProperties[mPropertyCount].mLiteralValue = nullptr;
    mProperties[mPropertyCount].mStringValue = nullptr;
    mProperties[mPropertyCount].mIntValue = value;
    mPropertyCount++;

    mLength += strlen(name) + 3 + 2;
  }

  void Format(std::string& result) {
    result.reserve(mLength + 2);

    result.push_back('{');
    for (int i = 0; i < mPropertyCount; i++) {
      result.push_back('"');
      result.append(mProperties[i].mName);
      result.push_back('"');
      result.push_back(':');

      if (mProperties[i].mLiteralValue) {
        result.push_back('"');
        result.append(mProperties[i].mLiteralValue);
        result.push_back('"');
      } else if (mProperties[i].mStringValue) {
        result.push_back('"');
        result.append(*mProperties[i].mStringValue);
        result.push_back('"');
      } else {
        result.push_back(mProperties[i].mIntValue + '0');
      }

      if (i + 1 != mPropertyCount) {
        result.push_back(',');
      }
    }

    result.push_back('}');
    result.push_back('\n');

    assert(result.length() == mLength + 2);
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
  std::map<FileID, FileInfo *> mFileMap;
  MangleContext *mMangleContext;
  ASTContext* mASTContext;

  typedef RecursiveASTVisitor<IndexConsumer> Super;

  struct AutoSetContext {
    AutoSetContext(IndexConsumer* self, NamedDecl* context)
     : mSelf(self), mPrev(self->mDeclContext), mDecl(context)
    {
      mSelf->mDeclContext = this;
    }

    ~AutoSetContext() {
      mSelf->mDeclContext = mPrev;
    }

    IndexConsumer* mSelf;
    AutoSetContext* mPrev;
    NamedDecl* mDecl;
  };

  AutoSetContext* mDeclContext;

  FileInfo *GetFileInfo(SourceLocation loc) {
    FileID id = sm.getFileID(loc);

    std::map<FileID, FileInfo *>::iterator it;
    it = mFileMap.find(id);
    if (it == mFileMap.end()) {
      // We haven't seen this file before. We need to make the FileInfo
      // structure information ourselves
      std::string filename = sm.getFilename(loc);
      const char *real = realpath(filename.c_str(), nullptr);
      std::string realstr(real ? real : filename.c_str());

      FileInfo *info = new FileInfo(realstr);
      it = mFileMap.insert(std::make_pair(id, info)).first;
    }
    return it->second;
  }

  // Helpers for processing declarations
  // Should we ignore this location?
  bool IsInterestingLocation(SourceLocation loc) {
    if (loc.isInvalid()) {
      return false;
    }

    return GetFileInfo(loc)->interesting;
  }

  std::string LocationToString(SourceLocation loc, size_t length = 0) {
    std::pair<FileID, unsigned> pair = sm.getDecomposedLoc(loc);

    bool isInvalid;
    unsigned line = sm.getLineNumber(pair.first, pair.second, &isInvalid);
    if (isInvalid) {
      return "";
    }
    unsigned column = sm.getColumnNumber(pair.first, pair.second, &isInvalid);
    if (isInvalid) {
      return "";
    }

    if (length) {
      return StringFormat("%d:%d-%d", line, column - 1, column - 1 + length);
    } else {
      return StringFormat("%d:%d", line, column - 1);
    }
  }

  // Returns the qualified name of `d` without considering template parameters.
  std::string GetQualifiedName(const NamedDecl* d) {
    const DeclContext* ctx = d->getDeclContext();
    if (ctx->isFunctionOrMethod()) {
      return d->getQualifiedNameAsString();
    }

    std::vector<const DeclContext *> contexts;

    // Collect contexts.
    while (ctx && isa<NamedDecl>(ctx)) {
      contexts.push_back(ctx);
      ctx = ctx->getParent();
    }

    std::string result;

    std::reverse(contexts.begin(), contexts.end());

    for (const DeclContext *dc : contexts) {
      if (const auto *spec = dyn_cast<ClassTemplateSpecializationDecl>(dc)) {
        result += spec->getNameAsString();

        if (spec->getSpecializationKind() == TSK_ExplicitSpecialization) {
          std::string backing;
          llvm::raw_string_ostream stream(backing);
          const TemplateArgumentList &templateArgs = spec->getTemplateArgs();
#if CLANG_VERSION_MAJOR > 3 || (CLANG_VERSION_MAJOR == 3 && CLANG_VERSION_MINOR >= 9)
          TemplateSpecializationType::PrintTemplateArgumentList(
            stream, templateArgs.asArray(), PrintingPolicy(ci.getLangOpts()));
#else
          TemplateSpecializationType::PrintTemplateArgumentList(
            stream, templateArgs.data(), templateArgs.size(), PrintingPolicy(ci.getLangOpts()));
#endif
          result += stream.str();
        }
      } else if (const auto *nd = dyn_cast<NamespaceDecl>(dc)) {
        if (nd->isAnonymousNamespace() || nd->isInline()) {
          continue;
        }
        result += nd->getNameAsString();
      } else if (const auto *rd = dyn_cast<RecordDecl>(dc)) {
        if (!rd->getIdentifier()) {
          result += "(anonymous)";
        } else {
          result += rd->getNameAsString();
        }
      } else if (const auto *fd = dyn_cast<FunctionDecl>(dc)) {
        result += fd->getNameAsString();
      } else if (const auto *ed = dyn_cast<EnumDecl>(dc)) {
        // C++ [dcl.enum]p10: Each enum-name and each unscoped
        // enumerator is declared in the scope that immediately contains
        // the enum-specifier. Each scoped enumerator is declared in the
        // scope of the enumeration.
        if (ed->isScoped() || ed->getIdentifier())
          result += ed->getNameAsString();
        else
          continue;
      } else {
        result += cast<NamedDecl>(dc)->getNameAsString();
      }
      result += "::";
    }

    if (d->getDeclName())
      result += d->getNameAsString();
    else
      result += "(anonymous)";

    return result;
  }

  std::string MangleLocation(SourceLocation loc) {
    FileInfo *f = GetFileInfo(loc);
    std::string filename = f->realname;
    return Hash(filename + std::string("@") + LocationToString(loc));
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

      return std::string("T_") + MangleQualifiedName(GetQualifiedName(decl));
    } else if (isa<NamespaceDecl>(decl) || isa<NamespaceAliasDecl>(decl)) {
      if (!decl->getIdentifier()) {
        // Anonymous.
        return std::string("NS_") + MangleLocation(decl->getLocation());
      }

      return std::string("NS_") + MangleQualifiedName(GetQualifiedName(decl));
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
   , mASTContext(nullptr)
   , mDeclContext(nullptr)
   , mTemplateStack(nullptr)
  {
    //ci.getDiagnostics().setClient(this, false);
    ci.getPreprocessor().addPPCallbacks(llvm::make_unique<PreprocessorHook>(this));
  }

  virtual DiagnosticConsumer *clone(DiagnosticsEngine &Diags) const {
    return new IndexConsumer(ci);
  }

  struct AutoTime {
    AutoTime(double* counter) : mCounter(counter), mStart(Time()) {}
    ~AutoTime() { if (mStart) { *mCounter += Time() - mStart; } }
    void stop() { *mCounter += Time() - mStart; mStart = 0; }
    double* mCounter;
    double mStart;
  };

  // All we need is to follow the final declaration.
  virtual void HandleTranslationUnit(ASTContext &ctx) {
    mMangleContext = clang::ItaniumMangleContext::create(ctx, ci.getDiagnostics());

    mASTContext = &ctx;
    TraverseDecl(ctx.getTranslationUnitDecl());

    // Emit all files now
    std::map<FileID, FileInfo *>::iterator it;
    for (it = mFileMap.begin(); it != mFileMap.end(); it++) {
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
      if (ftruncate(fd, length)) {
        return;
      }
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

  struct Context {
    std::string mName;
    std::vector<std::string> mSymbols;

    Context() {}
    Context(std::string name, std::vector<std::string> symbols) :
     mName(name), mSymbols(symbols) {}
  };

  bool TraverseEnumDecl(EnumDecl* d) {
    AutoSetContext asc(this, d);
    return Super::TraverseEnumDecl(d);
  }

  bool TraverseRecordDecl(RecordDecl* d) {
    AutoSetContext asc(this, d);
    return Super::TraverseRecordDecl(d);
  }
  bool TraverseCXXRecordDecl(CXXRecordDecl* d) {
    AutoSetContext asc(this, d);
    return Super::TraverseCXXRecordDecl(d);
  }

  bool TraverseFunctionDecl(FunctionDecl* d) {
    AutoSetContext asc(this, d);
    return Super::TraverseFunctionDecl(d);
  }
  bool TraverseCXXMethodDecl(CXXMethodDecl* d) {
    AutoSetContext asc(this, d);
    return Super::TraverseCXXMethodDecl(d);
  }
  bool TraverseCXXConstructorDecl(CXXConstructorDecl* d) {
    AutoSetContext asc(this, d);
    return Super::TraverseCXXConstructorDecl(d);
  }
  bool TraverseCXXConversionDecl(CXXConversionDecl* d) {
    AutoSetContext asc(this, d);
    return Super::TraverseCXXConversionDecl(d);
  }
  bool TraverseCXXDestructorDecl(CXXDestructorDecl* d) {
    AutoSetContext asc(this, d);
    return Super::TraverseCXXDestructorDecl(d);
  }

  Context TranslateContext(NamedDecl* d) {
    const FunctionDecl *f = dyn_cast<FunctionDecl>(d);
    if (f && f->isTemplateInstantiation()) {
      d = f->getTemplateInstantiationPattern();
    }

    std::vector<std::string> symbols = { GetMangledName(mMangleContext, d) };
    if (CXXMethodDecl::classof(d)) {
      symbols.clear();
      FindOverriddenMethods(dyn_cast<CXXMethodDecl>(d), symbols);
    }
    return Context(d->getQualifiedNameAsString(),
                   symbols);
  }

  Context GetContext(SourceLocation loc) {
    if (sm.isMacroBodyExpansion(loc)) {
      // If we're inside a macro definition, we don't return any context. It
      // will probably not be what the user expects if we do.
      return Context();
    }

    if (mDeclContext) {
      return TranslateContext(mDeclContext->mDecl);
    }
    return Context();
  }

  Context GetContext(Decl* d) {
    if (sm.isMacroBodyExpansion(d->getLocation())) {
      // If we're inside a macro definition, we don't return any context. It
      // will probably not be what the user expects if we do.
      return Context();
    }

    AutoSetContext* ctxt = mDeclContext;
    while (ctxt) {
      if (ctxt->mDecl != d) {
        return TranslateContext(ctxt->mDecl);
      }
      ctxt = ctxt->mPrev;
    }
    return Context();
  }

  static std::string ConcatSymbols(const std::vector<std::string> symbols) {
    if (symbols.empty()) {
      return "";
    }

    size_t total = 0;
    for (auto it = symbols.begin(); it != symbols.end(); it++) {
      total += it->length();
    }
    total += symbols.size() - 1;

    std::string symbolList;
    symbolList.reserve(total);

    for (auto it = symbols.begin(); it != symbols.end(); it++) {
      std::string symbol = *it;

      if (it != symbols.begin()) {
        symbolList.push_back(',');
      }
      symbolList.append(symbol);
    }

    return symbolList;
  }

  struct AutoTemplateContext {
    AutoTemplateContext(IndexConsumer* self)
     : mSelf(self)
     , mMode(Mode::GatherDependent)
     , mParent(self->mTemplateStack)
    {
      self->mTemplateStack = this;
    }

    ~AutoTemplateContext() {
      mSelf->mTemplateStack = mParent;
    }

    // We traverse templates in two modes:
    enum class Mode {
      // Gather mode does not traverse into specializations. It looks for
      // locations where it would help to have more info from template
      // specializations.
      GatherDependent,

      // Analyze mode traverses into template specializations and records
      // information about token locations saved in gather mode.
      AnalyzeDependent,
    };

    void VisitDependent(SourceLocation loc) {
      if (mMode == Mode::AnalyzeDependent) {
        return;
      }

      mDependentLocations.insert(loc.getRawEncoding());
      if (mParent) {
        mParent->VisitDependent(loc);
      }
    }

    bool NeedsAnalysis() const {
      if (!mDependentLocations.empty()) {
        return true;
      }
      if (mParent) {
        return mParent->NeedsAnalysis();
      }
      return false;
    }

    void SwitchMode() {
      mMode = Mode::AnalyzeDependent;
    }

    bool ShouldVisitTemplateInstantiations() const {
      if (mMode == Mode::AnalyzeDependent) {
        return true;
      }
      if (mParent) {
        return mParent->ShouldVisitTemplateInstantiations();
      }
      return false;
    }

    bool ShouldVisit(SourceLocation loc) {
      if (mMode == Mode::GatherDependent) {
        return true;
      }
      if (mDependentLocations.find(loc.getRawEncoding()) != mDependentLocations.end()) {
        return true;
      }
      if (mParent) {
        return mParent->ShouldVisit(loc);
      }
      return false;
    }

   private:
    IndexConsumer* mSelf;
    Mode mMode;
    std::unordered_set<unsigned> mDependentLocations;
    AutoTemplateContext* mParent;
  };

  AutoTemplateContext* mTemplateStack;

  bool shouldVisitTemplateInstantiations() const {
    if (mTemplateStack) {
      return mTemplateStack->ShouldVisitTemplateInstantiations();
    }
    return false;
  }

  bool TraverseClassTemplateDecl(ClassTemplateDecl* d) {
    AutoTemplateContext atc(this);
    Super::TraverseClassTemplateDecl(d);

    if (!atc.NeedsAnalysis()) {
      return true;
    }

    atc.SwitchMode();

    return Super::TraverseClassTemplateDecl(d);
  }

  bool TraverseFunctionTemplateDecl(FunctionTemplateDecl* d) {
    AutoTemplateContext atc(this);
    Super::TraverseFunctionTemplateDecl(d);

    if (!atc.NeedsAnalysis()) {
      return true;
    }

    atc.SwitchMode();

    return Super::TraverseFunctionTemplateDecl(d);
  }

  bool ShouldVisit(SourceLocation loc) {
    if (mTemplateStack) {
      return mTemplateStack->ShouldVisit(loc);
    }
    return true;
  }

  void VisitToken(const char *kind,
                  const char *syntaxKind,
                  std::string qualName,
                  SourceLocation loc,
                  const std::vector<std::string>& symbols,
                  Context context = Context(),
                  int flags = 0)
  {
    if (!ShouldVisit(loc)) {
      return;
    }

    unsigned startOffset = sm.getFileOffset(loc);
    unsigned endOffset = startOffset + Lexer::MeasureTokenLength(loc, sm, ci.getLangOpts());

    std::string locStr = LocationToString(loc, endOffset - startOffset);
    std::string rangeStr = LocationToString(loc, endOffset - startOffset);

    const char* startChars = sm.getCharacterData(loc);
    std::string text(startChars, endOffset - startOffset);

    FileInfo *f = GetFileInfo(loc);

    if (!IsValidToken(text)) {
      return;
    }

    std::string symbolList;

    size_t total = 0;
    for (auto it = symbols.begin(); it != symbols.end(); it++) {
      total += it->length();
    }
    total += symbols.size() - 1;

    symbolList.reserve(total);

    for (auto it = symbols.begin(); it != symbols.end(); it++) {
      std::string symbol = *it;

      if (!(flags & NO_CROSSREF)) {
        JSONFormatter fmt;

        fmt.Add("loc", locStr);
        fmt.Add("target", 1);
        fmt.Add("kind", kind);
        fmt.Add("pretty", qualName);
        fmt.Add("sym", symbol);
        if (!context.mName.empty()) {
          fmt.Add("context", context.mName);
        }
        std::string contextSymbol = ConcatSymbols(context.mSymbols);
        if (!contextSymbol.empty()) {
          fmt.Add("contextsym", contextSymbol);
        }

        std::string s;
        fmt.Format(s);
        f->output.push_back(std::move(s));
      }

      if (it != symbols.begin()) {
        symbolList.push_back(',');
      }
      symbolList.append(symbol);
    }

    JSONFormatter fmt;

    fmt.Add("loc", rangeStr);
    fmt.Add("source", 1);

    std::string syntax;
    if (flags & NO_CROSSREF) {
      fmt.Add("syntax", "");
    } else {
      syntax = kind;
      syntax.push_back(',');
      syntax.append(syntaxKind);
      fmt.Add("syntax", syntax);
    }

    std::string pretty(syntaxKind);
    pretty.push_back(' ');
    pretty.append(qualName);
    fmt.Add("pretty", pretty);

    fmt.Add("sym", symbolList);

    if (flags & NO_CROSSREF) {
      fmt.Add("no_crossref", 1);
    }

    std::string buf;
    fmt.Format(buf);
    f->output.push_back(std::move(buf));
  }

  void VisitToken(const char *kind,
                  const char *syntaxKind,
                  std::string qualName,
                  SourceLocation loc,
                  std::string symbol,
                  Context context = Context(),
                  int flags = 0)
  {
    std::vector<std::string> v = { symbol };
    VisitToken(kind, syntaxKind, qualName, loc, v, context, flags);
  }

  void NormalizeLocation(SourceLocation* loc) {
    *loc = sm.getSpellingLoc(*loc);
  }

  bool VisitNamedDecl(NamedDecl *d) {
    SourceLocation loc = d->getLocation();
    NormalizeLocation(&loc);
    if (!IsInterestingLocation(loc)) {
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

    // For destructors, loc points to the ~ character. We want to skip to the
    // class name.
    if (isa<CXXDestructorDecl>(d)) {
      const char* p = sm.getCharacterData(loc);
      assert(*p == '~');
      p++;

      unsigned skipped = 1;
      while (*p == ' ' || *p == '\t' || *p == '\r' || *p == '\n') {
        p++;
        skipped++;
      }

      loc = loc.getLocWithOffset(skipped);

      prettyKind = "destructor";
    }

    VisitToken(kind, prettyKind, d->getQualifiedNameAsString(), loc, symbols, GetContext(d), flags);

    return true;
  }

  bool VisitCXXConstructExpr(CXXConstructExpr* e) {
    SourceLocation loc = e->getLocStart();
    NormalizeLocation(&loc);
    if (!IsInterestingLocation(loc)) {
      return true;
    }

    FunctionDecl* ctor = e->getConstructor();
    if (ctor->isTemplateInstantiation()) {
      ctor = ctor->getTemplateInstantiationPattern();
    }
    std::string mangled = GetMangledName(mMangleContext, ctor);

    // FIXME: Need to do something different for list initialization.

    VisitToken("use", "constructor", ctor->getQualifiedNameAsString(), loc, mangled, GetContext(loc));

    return true;
  }

  bool VisitCallExpr(CallExpr *e) {
    Decl *callee = e->getCalleeDecl();
    if (!callee || !FunctionDecl::classof(callee)) {
      return true;
    }

    const NamedDecl *namedCallee = dyn_cast<NamedDecl>(callee);

    SourceLocation startLoc = callee->getLocStart();
    SourceLocation loc = startLoc;
    NormalizeLocation(&loc);
    if (!IsInterestingLocation(loc)) {
      return true;
    }

    const FunctionDecl *f = dyn_cast<FunctionDecl>(namedCallee);
    if (f->isTemplateInstantiation()) {
      namedCallee = f->getTemplateInstantiationPattern();
    }

    std::string mangled = GetMangledName(mMangleContext, namedCallee);

    Expr* calleeExpr = e->getCallee()->IgnoreParenImpCasts();

    if (CXXOperatorCallExpr::classof(e)) {
      // Just take the first token.
      CXXOperatorCallExpr* op = dyn_cast<CXXOperatorCallExpr>(e);
      loc = op->getOperatorLoc();
      NormalizeLocation(&loc);
    } else if (MemberExpr::classof(calleeExpr)) {
      MemberExpr* member = dyn_cast<MemberExpr>(calleeExpr);
      loc = member->getMemberLoc();
      NormalizeLocation(&loc);
    } else if (DeclRefExpr::classof(calleeExpr)) {
      // We handle this in VisitDeclRefExpr.
      return true;
    } else {
      if (callee->getLocEnd() != startLoc) {
        // Skip this call. If we can't find a single token, we don't have a
        // good UI for displaying the call.
        return true;
      }
    }

    VisitToken("use", "function", namedCallee->getQualifiedNameAsString(), loc, mangled, GetContext(loc));

    return true;
  }

  bool VisitTagTypeLoc(TagTypeLoc l) {
    SourceLocation loc = l.getBeginLoc();
    NormalizeLocation(&loc);
    if (!IsInterestingLocation(loc)) {
      return true;
    }

    TagDecl* decl = l.getDecl();
    std::string mangled = GetMangledName(mMangleContext, decl);
    VisitToken("use", "type", decl->getQualifiedNameAsString(), loc, mangled, GetContext(loc));
    return true;
  }

  bool VisitTypedefTypeLoc(TypedefTypeLoc l) {
    SourceLocation loc = l.getBeginLoc();
    NormalizeLocation(&loc);
    if (!IsInterestingLocation(loc)) {
      return true;
    }

    NamedDecl* decl = l.getTypedefNameDecl();
    std::string mangled = GetMangledName(mMangleContext, decl);
    VisitToken("use", "type", decl->getQualifiedNameAsString(), loc, mangled, GetContext(loc));
    return true;
  }

  bool VisitInjectedClassNameTypeLoc(InjectedClassNameTypeLoc l) {
    SourceLocation loc = l.getBeginLoc();
    NormalizeLocation(&loc);
    if (!IsInterestingLocation(loc)) {
      return true;
    }

    NamedDecl* decl = l.getDecl();
    std::string mangled = GetMangledName(mMangleContext, decl);
    VisitToken("use", "type", decl->getQualifiedNameAsString(), loc, mangled, GetContext(loc));
    return true;
  }

  bool VisitTemplateSpecializationTypeLoc(TemplateSpecializationTypeLoc l) {
    SourceLocation loc = l.getBeginLoc();
    NormalizeLocation(&loc);
    if (!IsInterestingLocation(loc)) {
      return true;
    }

    TemplateDecl* td = l.getTypePtr()->getTemplateName().getAsTemplateDecl();
    if (ClassTemplateDecl *d = dyn_cast<ClassTemplateDecl>(td)) {
      NamedDecl* decl = d->getTemplatedDecl();
      std::string mangled = GetMangledName(mMangleContext, decl);
      VisitToken("use", "type", decl->getQualifiedNameAsString(), loc, mangled, GetContext(loc));
    }

    return true;
  }

  bool VisitDeclRefExpr(DeclRefExpr *e) {
    SourceLocation loc = e->getExprLoc();
    NormalizeLocation(&loc);
    if (!IsInterestingLocation(loc)) {
      return true;
    }

    if (e->hasQualifier()) {
      loc = e->getNameInfo().getLoc();
      NormalizeLocation(&loc);
    }

    NamedDecl* decl = e->getDecl();
    if (const VarDecl* d2 = dyn_cast<VarDecl>(decl)) {
      int flags = 0;
      if (d2->isLocalVarDeclOrParm()) {
        flags = NO_CROSSREF;
      }
      std::string mangled = GetMangledName(mMangleContext, decl);
      VisitToken("use", "variable", decl->getQualifiedNameAsString(), loc, mangled, GetContext(loc), flags);
    } else if (isa<FunctionDecl>(decl)) {
      const FunctionDecl *f = dyn_cast<FunctionDecl>(decl);
      if (f->isTemplateInstantiation()) {
        decl = f->getTemplateInstantiationPattern();
      }

      std::string mangled = GetMangledName(mMangleContext, decl);
      VisitToken("use", "function", decl->getQualifiedNameAsString(), loc, mangled, GetContext(loc));
    } else if (isa<EnumConstantDecl>(decl)) {
      std::string mangled = GetMangledName(mMangleContext, decl);
      VisitToken("use", "enum", decl->getQualifiedNameAsString(), loc, mangled, GetContext(loc));
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

      SourceLocation loc = ci->getMemberLocation();
      NormalizeLocation(&loc);
      if (!IsInterestingLocation(loc)) {
        continue;
      }

      FieldDecl* member = ci->getMember();
      std::string mangled = GetMangledName(mMangleContext, member);
      VisitToken("use", "field", member->getQualifiedNameAsString(), loc, mangled, GetContext(d));
    }

    return true;
  }

  bool VisitMemberExpr(MemberExpr* e) {
    SourceLocation loc = e->getExprLoc();
    NormalizeLocation(&loc);
    if (!IsInterestingLocation(loc)) {
      return true;
    }

    ValueDecl* decl = e->getMemberDecl();
    if (FieldDecl* field = dyn_cast<FieldDecl>(decl)) {
      std::string mangled = GetMangledName(mMangleContext, field);
      VisitToken("use", "field", field->getQualifiedNameAsString(), loc, mangled, GetContext(loc));
    }
    return true;
  }

  bool VisitCXXDependentScopeMemberExpr(CXXDependentScopeMemberExpr* e) {
    SourceLocation loc = e->getMemberLoc();
    NormalizeLocation(&loc);
    if (!IsInterestingLocation(loc)) {
      return true;
    }

    if (mTemplateStack) {
      mTemplateStack->VisitDependent(loc);
    }
    return true;
  }

  void MacroDefined(const Token &tok, const MacroDirective *macro) {
    if (macro->getMacroInfo()->isBuiltinMacro()) {
      return;
    }
    SourceLocation loc = tok.getLocation();
    NormalizeLocation(&loc);
    if (!IsInterestingLocation(loc)) {
      return;
    }

    IdentifierInfo* ident = tok.getIdentifierInfo();
    if (ident) {
      std::string mangled = std::string("M_") + MangleLocation(loc);
      VisitToken("def", "macro", ident->getName(), loc, mangled);
    }
  }

  void MacroUsed(const Token &tok, const MacroInfo *macro) {
    if (!macro) {
      return;
    }
    if (macro->isBuiltinMacro()) {
      return;
    }
    SourceLocation loc = tok.getLocation();
    NormalizeLocation(&loc);
    if (!IsInterestingLocation(loc)) {
      return;
    }

    IdentifierInfo* ident = tok.getIdentifierInfo();
    if (ident) {
      std::string mangled = std::string("M_") + MangleLocation(macro->getDefinitionLoc());
      VisitToken("use", "macro", ident->getName(), loc, mangled);
    }
  }
};

void
PreprocessorHook::MacroDefined(const Token &tok, const MacroDirective* md)
{
  indexer->MacroDefined(tok, md);
}

void
PreprocessorHook::MacroExpands(const Token &tok, const MacroDefinition& md,
                               SourceRange range, const MacroArgs *ma)
{
  indexer->MacroUsed(tok, md.getMacroInfo());
}

void
PreprocessorHook::MacroUndefined(const Token &tok, const MacroDefinition& md)
{
  indexer->MacroUsed(tok, md.getMacroInfo());
}

void
PreprocessorHook::Defined(const Token &tok, const MacroDefinition& md, SourceRange range)
{
  indexer->MacroUsed(tok, md.getMacroInfo());
}

void
PreprocessorHook::Ifdef(SourceLocation loc, const Token &tok, const MacroDefinition& md)
{
  indexer->MacroUsed(tok, md.getMacroInfo());
}

void
PreprocessorHook::Ifndef(SourceLocation loc, const Token &tok, const MacroDefinition& md)
{
  indexer->MacroUsed(tok, md.getMacroInfo());
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
