// A query with a path (`path:inpath/inside pathprobe`) finds this file by
// its name, and PathProbe's definition and uses in inpath/inside, but not
// inpath/outside's (see test_QueryPath).

int PathProbe() { return 1; }

int InsideCaller() { return PathProbe(); }
