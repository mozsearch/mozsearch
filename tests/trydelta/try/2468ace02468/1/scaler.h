// A class with a version of its own for each version of a library a file is
// compiled for (like Firefox's FFmpeg decoders), each an explicit
// specialization.
namespace trydelta {
template <int V>
class Scaler {};
template <>
class Scaler<SCALER_VERSION> {
 public:
  static int Scale(int aValue);
};
}  // namespace trydelta
