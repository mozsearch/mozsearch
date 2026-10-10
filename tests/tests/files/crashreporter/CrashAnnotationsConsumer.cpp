#include "bindings/CrashAnnotations.h"

CrashReporter::Annotation CrashAnnotationsConsumer() {
  return CrashReporter::Annotation::AbortMessage;
}
