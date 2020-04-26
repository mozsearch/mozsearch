var Panel = new (class Panel {
  constructor() {
    this.panel = document.getElementById("panel");
    this.toggleButton = document.getElementById("panel-toggle");
    this.icon = this.panel.querySelector(".navpanel-icon");
    this.content = document.getElementById("panel-content");
    this.accelEnabledCheckbox = document.getElementById("panel-accel-enable");

    this.permalinkNode = this.findLink("Permalink");
    this.logNode = this.findLink("Log");
    this.rawNode = this.findLink("Raw");

    this.toggleButton.addEventListener("click", () => this.toggle());
    this.accelEnabledCheckbox.addEventListener("change", () => {
      localStorage.setItem("accel-enable", event.target.checked ? "1" : "0");
      this.updateAccelerators();
    });
    document.documentElement.addEventListener("keypress", event =>
      this.maybeHandleAccelerator(event)
    );

    if (this.permalinkNode) {
      this.permalinkNode.addEventListener("click", event => {
        if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
          return;
        }
        window.history.pushState(
          { permalink: event.target.href },
          window.title,
          event.target.href
        );
        event.preventDefault();
      });
    }

    // If the user toggles it in a different tab, update the checkbox/state here
    //
    // TODO(emilio): We should probably do the same for the case-sensitive
    // checkbox and such.
    window.addEventListener("storage", () => this.initFromLocalStorage());

    this.initFromLocalStorage();
  }

  get acceleratorsEnabled() {
    return this.accelEnabledCheckbox.checked;
  }

  initFromLocalStorage() {
    let acceleratorsEnabled =
      !("accel-enable" in localStorage) ||
      localStorage.getItem("accel-enable") == "1";
    this.accelEnabledCheckbox.checked = acceleratorsEnabled;
    this.updateAccelerators();
  }

  updateAccelerators() {
    let enabled = this.acceleratorsEnabled;
    for (let accel of this.panel.querySelectorAll("span.accel")) {
      accel.style.display = enabled ? "" : "none";
    }
  }

  findLink(title) {
    return this.panel.querySelector(`a[title="${title}"]`);
  }

  maybeHandleAccelerator(event) {
    if (!this.acceleratorsEnabled) {
      return;
    }
    if (event.altKey || event.ctrlKey || event.metaKey) {
      return;
    }
    var inputs = /input|select|textarea/i;
    if (inputs.test(event.target.nodeName)) {
      return;
    }
    let link = (() => {
      switch (event.key) {
        case "y":
        case "Y":
          return this.permalinkNode;
        case "l":
        case "L":
          return this.logNode;
        case "r":
        case "R":
          return this.rawNode;
      }
    })();

    if (link) {
      link.click();
      event.preventDefault();
    }
  }

  toggle() {
    let hidden = this.content.style.display != "none";
    this.content.style.display = hidden ? "none" : "";
    this.content.setAttribute("aria-hidden", hidden);
    this.content.setAttribute("aria-expanded", !hidden);
    this.icon.classList.toggle("expanded");
  }
})();
