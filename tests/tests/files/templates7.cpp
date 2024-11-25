// Testcase proposed by Botond on
// https://bugzilla.mozilla.org/show_bug.cgi?id=1833695

struct Theme {
  void CreateWebRenderCommandsForWidget() {
    OutOfLineTemplateShouldntHaveContextSym(0);
    OutOfLineShouldntHaveContextSym(0);
    InlineTemplateShouldHaveContextSym(0);
    InlineShouldHaveContextSym(0);
  }

  template <typename T>
  void OutOfLineTemplateShouldntHaveContextSym(T);

  void OutOfLineShouldntHaveContextSym(int);

  template <typename T>
  inline void InlineTemplateShouldHaveContextSym(T) {}

  inline void InlineShouldHaveContextSym(int) {}
};

template <typename T>
void Theme::OutOfLineTemplateShouldntHaveContextSym(T) {}

void Theme::OutOfLineShouldntHaveContextSym(int) {}
