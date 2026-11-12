/**
 * A method's overrides calling the method they override (super calls, which
 * aren't virtual), and a call of it which could be a virtual call of any of
 * them, for /query/'s results of a method in an override hierarchy (see
 * test_QueryRelations.js).
 **/

class SuperBase {
 public:
  virtual void teardown() {}
};

class SuperMid : public SuperBase {
 public:
  void teardown() override { SuperBase::teardown(); }
};

class SuperLeaf : public SuperMid {
 public:
  void teardown() override { SuperMid::teardown(); }
};

class SuperCousin : public SuperBase {
 public:
  void teardown() override { SuperBase::teardown(); }
};

void tearDownAll(SuperBase* base) { base->teardown(); }
