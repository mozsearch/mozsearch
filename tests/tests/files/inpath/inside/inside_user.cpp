// A use of PathProbe in the path the query names, by a file whose name doesn't
// have "pathprobe" (see test_QueryPath).

int PathProbe();

int InsideUser() { return PathProbe() + 1; }
