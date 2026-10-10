#ifndef CrashAnnotations_h
#define CrashAnnotations_h

#include <stdint.h>

namespace CrashReporter {

// Typed enum representing all crash annotations
enum class Annotation : uint32_t {
  AbortMessage = 0,
  AsyncShutdownTimeout = 1,
  Count = 2
};

}  // namespace CrashReporter

#endif  // CrashAnnotations_h
