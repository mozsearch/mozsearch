// Observer service topics (see
// tools/languages/semantic_mappings/observer_topics_cpp.toml), by literals and
// by constants, with stand-ins for nsIObserverService and nsISupports.

#define NS_TEST_SHUTDOWN_TOPIC "test-shutdown"

extern "C" int strcmp(const char* aA, const char* aB);

namespace observer_test {

class Supports {};

class ObserverService {
 public:
  void NotifyObservers(Supports* aSubject, const char* aTopic,
                       const char16_t* aData);
  void AddObserver(Supports* aObserver, const char* aTopic, bool aOwnsWeak);
  void RemoveObserver(Supports* aObserver, const char* aTopic);
};

class Observer : public Supports {
 public:
  void Register(ObserverService* aObs) {
    aObs->AddObserver(this, NS_TEST_SHUTDOWN_TOPIC, false);
    aObs->AddObserver(this, "test-literal", false);
  }

  int Observe(Supports* aSubject, const char* aTopic, const char16_t* aData) {
    if (!strcmp(aTopic, "test-literal")) {
      return 1;
    }
    if (!strcmp(aTopic, NS_TEST_SHUTDOWN_TOPIC)) {
      return 2;
    }
    return 0;
  }
};

void Shutdown(ObserverService* aObs) {
  aObs->NotifyObservers(nullptr, NS_TEST_SHUTDOWN_TOPIC, nullptr);
}

}  // namespace observer_test
