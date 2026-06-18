// Without the workaround for https://github.com/llvm/llvm-project/issues/200336 in MozsearchIndexer.cpp, this crashes the Clang plugin.
template <class T>
concept C = requires(const T &t) {
    []<class U>(const U &) {}(t);
};
