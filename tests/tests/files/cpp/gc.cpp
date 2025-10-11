namespace GC {

void DoGC() {
}

bool CanGC(int foo) {
  DoGC();
  return true;
}

void CanGC2() {
  (void) CanGC(10);
}

bool CannotGC() {
  return true;
}

}
