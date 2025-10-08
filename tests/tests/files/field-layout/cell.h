namespace js {

namespace gc {

struct Cell {
  void* header_;
  void doCell() {}

  void Hello() {
    header_ = this;
  }
};

class TenuredCell : public Cell {
  void* fieldTenuredCell_;
  void doTenuredCell() {}

  void Hello() {
    header_ = this + 1;
  }
};

template <class PtrT>
class TenuredCellWithNonGCPointer : public TenuredCell {
  void* fTenuredCellWithNonGCPointer_;
  void doTenuredCellWithNonGCPointer() {}
};

template <class BaseCell, class PtrT>
class CellWithTenuredGCPointer : public BaseCell {
  void* fCellWithTenuredGCPointer;
  void doCellWithTenuredGCPointer() {
    this->Hello();
  }
};

}  // namespace gc

}  // namespace js
