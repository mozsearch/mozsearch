#include <stdio.h>

class CallerOne;
class CallerTwo;
class CallerThree;
class CallerFour;

class CallerOne {
 public:
  void one_calls_two_left(CallerTwo* two, CallerThree* three, CallerFour* four);

  void one_calls_two_right(CallerTwo* two, CallerThree* three,
                           CallerFour* four);
};

class CallerTwo {
 public:
  void two_left_calls_three_nexus(CallerThree* three, CallerFour* four);

  void two_right_calls_three_nexus(CallerThree* three, CallerFour* four);
};

class CallerThree {
 public:
  void three_nexus(CallerFour* four);
};

class CallerFour {
 public:
  void four_left() { printf("four_left\n"); };

  void four_right() { printf("four_right\n"); }
};

void CallerOne::one_calls_two_left(CallerTwo* two, CallerThree* three,
                                   CallerFour* four) {
  two->two_left_calls_three_nexus(three, four);
}

void CallerOne::one_calls_two_right(CallerTwo* two, CallerThree* three,
                                    CallerFour* four) {
  two->two_right_calls_three_nexus(three, four);
}

void CallerTwo::two_left_calls_three_nexus(CallerThree* three,
                                           CallerFour* four) {
  three->three_nexus(four);
}

void CallerTwo::two_right_calls_three_nexus(CallerThree* three,
                                            CallerFour* four) {
  three->three_nexus(four);
}

void CallerThree::three_nexus(CallerFour* four) {
  four->four_left();
  four->four_right();
}

int main(void) {
  CallerOne one;
  CallerTwo two;
  CallerThree three;
  CallerFour four;

  one.one_calls_two_left(&two, &three, &four);
  one.one_calls_two_right(&two, &three, &four);

  return 0;
}
