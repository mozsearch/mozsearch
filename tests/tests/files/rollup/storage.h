// A second pair of subsystems calling each other (with model.h and view.cpp's
// pair), for the whole tree's block diagram's groups of subsystems (see
// cmd_block_diagram's `louvain`): Rollup Engine calls Rollup Storage.

namespace rollup {

class Storage {
 public:
  void Put(int aValue);
  int Get() const;

 private:
  int mValue = 0;
};

}  // namespace rollup
