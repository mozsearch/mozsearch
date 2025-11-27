namespace range_with_macro {

#define MOZ_UNANNOTATED __attribute__((annotate("moz_unannotated")))

MOZ_UNANNOTATED static const int withMacro[] = {
  1, 2, 3
};

}
