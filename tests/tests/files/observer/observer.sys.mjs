// Observer service topics (see
// tools/languages/semantic_mappings/observer_topics_js.toml), by literals and
// by constants.

const TEST_TOPIC = "test-js-topic";

export const Observer = {
  init() {
    Services.obs.addObserver(this, TEST_TOPIC);
    Services.obs.addObserver(this, "test-literal");
  },

  observe(subject, topic, data) {
    switch (topic) {
      case TEST_TOPIC:
        break;
      case "test-literal":
        break;
    }
  },

  notify() {
    Services.obs.notifyObservers(null, "test-literal");
  },
};
