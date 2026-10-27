// The definition of each version's Scaler::Scale, the same code for each
// (see scaler2.cpp and scaler3.cpp).
namespace trydelta {
int Scaler<SCALER_VERSION>::Scale(int aValue) {
  return aValue * SCALER_VERSION;
}
}  // namespace trydelta
