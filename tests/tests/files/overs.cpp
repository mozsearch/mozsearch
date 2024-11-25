/**
 * This file is intended to create interesting test cases for searches against
 * a base class with a limited number of overrides.
 **/

class DoubleBase {
 public:
  virtual void doublePure() = 0;
};

class DoubleSubOne : public DoubleBase {
 public:
  void doublePure() override {
    // Sub one.
  }
};

class DoubleSubTwo : public DoubleBase {
 public:
  void doublePure() override {
    // Sub two.
  }
};

class TripleBase {
 public:
  virtual void triplePure() = 0;
};

class TripleSubOne : public TripleBase {
 public:
  void triplePure() override {
    // Triple sub one.
  }
};

class TripleSubTwo : public TripleBase {
 public:
  void triplePure() override {
    // Triple sub two.
  }
};

class TripleSubThree : public TripleBase {
 public:
  void triplePure() override {
    // Triple sub three.
  }
};

void generateDoubleUses(void) {
  DoubleBase* subOne = new DoubleSubOne();
  DoubleBase* subTwo = new DoubleSubTwo();
  DoubleSubOne explicitOne;
  DoubleSubTwo explicitTwo;

  subOne->doublePure();
  subTwo->doublePure();

  explicitOne.doublePure();
  explicitTwo.doublePure();
}

void generateTripleUses(void) {
  TripleBase* subOne = new TripleSubOne();
  TripleBase* subTwo = new TripleSubTwo();
  TripleBase* subThree = new TripleSubThree();
  TripleSubOne explicitOne;
  TripleSubTwo explicitTwo;
  TripleSubThree explicitThree;

  subOne->triplePure();
  subTwo->triplePure();
  subThree->triplePure();

  explicitOne.triplePure();
  explicitTwo.triplePure();
  explicitThree.triplePure();
}
