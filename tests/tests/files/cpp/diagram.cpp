namespace diagram {

namespace uses_lines_local {

void target() {}

void caller0() { target(); }
void caller1() { target(); }
void caller2() { target(); }
void caller3() { target(); }
void caller4() { target(); }
void caller5() { target(); }
void caller6() { target(); }
void caller7() { target(); }
void caller8() { target(); }
void caller9() { target(); }

void caller10() { target(); }
void caller11() { target(); }
void caller12() { target(); }
void caller13() { target(); }
void caller14() { target(); }
void caller15() { target(); }
void caller16() { target(); }
void caller17() { target(); }
void caller18() { target(); }
void caller19() { target(); }

}

namespace uses_lines_global {

void target() {}

#include "cpp/diagram_callers_1.h"
#include "cpp/diagram_callers_2.h"

}

namespace uses_paths {

void target() {}

#include "cpp/diagram_caller_0.h"
#include "cpp/diagram_caller_1.h"
#include "cpp/diagram_caller_2.h"
#include "cpp/diagram_caller_3.h"
#include "cpp/diagram_caller_4.h"
#include "cpp/diagram_caller_5.h"
#include "cpp/diagram_caller_6.h"
#include "cpp/diagram_caller_7.h"
#include "cpp/diagram_caller_8.h"
#include "cpp/diagram_caller_9.h"
#include "cpp/diagram_caller_10.h"
#include "cpp/diagram_caller_11.h"
#include "cpp/diagram_caller_12.h"
#include "cpp/diagram_caller_13.h"
#include "cpp/diagram_caller_14.h"
#include "cpp/diagram_caller_15.h"
#include "cpp/diagram_caller_16.h"
#include "cpp/diagram_caller_17.h"
#include "cpp/diagram_caller_18.h"
#include "cpp/diagram_caller_19.h"

}

}
