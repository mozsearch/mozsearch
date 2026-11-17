// A file whose name has "pathprobe", and a use of PathProbe, outside the path
// the query names (see test_QueryPath).

int PathProbe();

int OutsideUser() { return PathProbe() + 2; }
