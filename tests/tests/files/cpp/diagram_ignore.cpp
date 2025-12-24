namespace diagram_ignore {

void F1() {
}

void F2() {
  F1();
}

void F3() {
  F2();
}

void F4() {
  F1();
}

void F5() {
  F3();
  F4();
}

void F6() {
  F5();
}

void F7() {
  F6();
}

void F8() {
  F2();
}

void F9() {
  F7();
  F8();
}

void F10() {
  F9();
}

}
