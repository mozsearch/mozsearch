interface Test {
  any argumentNameKeyword(any async,
                          any attribute,
                          any callback,
                          any const,
                          any constructor,
                          any deleter,
                          any dictionary,
                          any enum,
                          any getter,
                          any includes,
                          any inherit,
                          any interface,
                          any iterable,
                          any maplike,
                          any mixin,
                          any namespace,
                          any partial,
                          any readonly,
                          any required,
                          any setlike,
                          any setter,
                          any static,
                          any stringifier,
                          any typedef,
                          any unrestricted);

  attribute any async;
  attribute any required;

  getter any(unsigned long arg);

  // WebIDL.py doesn't support includes as operation name.
  any _includes();
};
