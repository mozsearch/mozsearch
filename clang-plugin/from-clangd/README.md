The facilities in this subdirectory are copied over from clangd
(https://clangd.llvm.org/).

The files here are currently copies of the following upstream files:
https://github.com/llvm/llvm-project/blob/2bcbcbefcd0f7432f99cc07bb47d1e1ecb579a3f/clang-tools-extra/clangd/HeuristicResolver.h
https://github.com/llvm/llvm-project/blob/2bcbcbefcd0f7432f99cc07bb47d1e1ecb579a3f/clang-tools-extra/clangd/HeuristicResolver.cpp

Starting with Clang 20, these facilities are exposed in the clang API
headers. We can drop them when we stop supporting Clang < 20.
