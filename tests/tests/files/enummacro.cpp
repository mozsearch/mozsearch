// https://bugzilla.mozilla.org/show_bug.cgi?id=1282172
enum ArenaObjectID {
  eArenaObjectID_DummyBeforeFirstObjectID = 0,

#define PRES_ARENA_OBJECT(name_) eArenaObjectID_##name_,
#include "enummacro.h"
#undef PRES_ARENA_OBJECT

  eArenaObjectID_COUNT
};

void someFunction() {
  ArenaObjectID useTheEnum = ArenaObjectID::eArenaObjectID_nsRuleNode;
}
