browser.omnibox.onInputChanged.addListener((text, suggest) => {
  console.log("ENTERED", text);

  if (text.length < 3) {
    return;
  }

  function reqListener() {
    console.log("RESPONSE", this.responseText);
    let response = JSON.parse(this.responseText);
    suggest(response.map(result => { return { content: "symbol:" + result.symbol, description: result.id};  }));
  }

  let xhr = new XMLHttpRequest();
  xhr.addEventListener("load", reqListener);
  xhr.open("GET", "http://localhost:8001/mozilla-central/complete/" + text, true);
  xhr.send();
});

browser.omnibox.onInputEntered.addListener((text, disposition) => {
  let url = "https://searchfox.org/mozilla-central/search?q=" + encodeURIComponent(text);
  switch (disposition) {
    case "currentTab":
      browser.tabs.update({url});
      break;
    case "newForegroundTab":
      browser.tabs.create({url});
      break;
    case "newBackgroundTab":
      browser.tabs.create({url, active: false});
      break;
  }
});
