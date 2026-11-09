namespace trydelta {

#include "sprockets.inc"

int SprocketTeeth(bool aLarge) {
  return aLarge ? kLargeSprocketTeeth : kSmallSprocketTeeth;
}

}  // namespace trydelta
