// A C++ class named like js/shared_class_name.js's JS class: the diagrams'
// node of its methods' class is it, by their records' `parent_sym`, rather
// than the first symbol named SharedClassName, the JS class's (see
// `ancestors` in tools/src/cmd_pipeline/symbol_graph.rs).

class SharedClassName {
 public:
  void Method() {}
};

// A namespace named like js/shared_class_name.js's JS variable: the
// diagrams' node of the namespace its class and function are in is it, by
// their records' `parentsym`s (and crossref's record of the namespace),
// rather than the first symbol named shared_ns, the JS variable's (bug
// 1958114).
namespace shared_ns {

class Inner {
 public:
  void Method() {}
};

void Free() {}

}  // namespace shared_ns

void ClassNodesCaller() {
  SharedClassName named;
  named.Method();
  shared_ns::Inner inner;
  inner.Method();
  shared_ns::Free();
}
