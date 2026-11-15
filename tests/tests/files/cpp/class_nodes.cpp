// A C++ class named like js/shared_class_name.js's JS class: the diagrams'
// node of its methods' class is it, by their records' `parent_sym`, rather
// than the first symbol named SharedClassName, the JS class's (see
// `ancestors` in tools/src/cmd_pipeline/symbol_graph.rs).

class SharedClassName {
 public:
  void Method() {}
};

void ClassNodesCaller() {
  SharedClassName named;
  named.Method();
}
