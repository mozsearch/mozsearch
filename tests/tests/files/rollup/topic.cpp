// Observer topics in call roll-ups (see crossref's `call_edges`): notifying
// observers of a topic is, for roll-ups, a call of the observers' methods
// receiving it, so Announcer calls Listener.

extern "C" int strcmp(const char* aA, const char* aB);

namespace rollup {

class TopicService {
 public:
  void NotifyObservers(void* aSubject, const char* aTopic,
                       const char16_t* aData);
};

class Announcer {
 public:
  void Announce(TopicService* aService) {
    aService->NotifyObservers(nullptr, "rollup-changed", nullptr);
  }
};

class Listener {
 public:
  int Observe(void* aSubject, const char* aTopic, const char16_t* aData) {
    if (!strcmp(aTopic, "rollup-changed")) {
      return 1;
    }
    return 0;
  }
};

}  // namespace rollup
