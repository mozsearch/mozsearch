#ifdef TEST_MACRO1
#ifdef TEST_MACRO2

int x = 12;

#endif
#endif

#define EMPTY_MACRO
#define CONST_MACRO 15
#define IDENT_MACRO(Arg) Arg
#define MULTI_LINE_MACRO(Name, Value) \
    static bool Name() \
    { \
        return Value; \
    }
#define NESTED_MACRO CONST_MACRO
#define NESTED_MACRO_WITH_ARG(Arg) IDENT_MACRO(Arg)

EMPTY_MACRO
EMPTY_MACRO int i = CONST_MACRO; EMPTY_MACRO
EMPTY_MACRO int j = IDENT_MACRO(16); EMPTY_MACRO
EMPTY_MACRO int k = IDENT_MACRO(IDENT_MACRO(17)); EMPTY_MACRO
EMPTY_MACRO int l = NESTED_MACRO; EMPTY_MACRO
EMPTY_MACRO int m = NESTED_MACRO_WITH_ARG(18); EMPTY_MACRO
EMPTY_MACRO int n = NESTED_MACRO_WITH_ARG(CONST_MACRO); EMPTY_MACRO
EMPTY_MACRO int o = NESTED_MACRO_WITH_ARG(IDENT_MACRO(EMPTY_MACRO 19 EMPTY_MACRO)) EMPTY_MACRO;

MULTI_LINE_MACRO(Bool0, true)
MULTI_LINE_MACRO(Bool1, true) MULTI_LINE_MACRO(Bool2, false)

MULTI_LINE_MACRO(
    Bool3,
    false
)

#if defined(TARGET_linux64)
#define PER_TARGET_FUNCTION bool per_target_function() { int a; int b; d = 5; } int f = per_target_function();
#elif defined(TARGET_macosx64)
#define PER_TARGET_FUNCTION bool per_target_function() { int b; int a; b = 2; } int f = per_target_function();
#elif defined(TARGET_win64)
#define PER_TARGET_FUNCTION bool per_target_function() { int c; d = 3; } int f = per_target_function();
#endif

int d;

PER_TARGET_FUNCTION
int g = per_target_function();

#include TEST_MACRO_INCLUDE
