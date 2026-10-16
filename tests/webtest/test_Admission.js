"use strict";

// The web-server's admission (see tools/src/admission.rs): a client (by its
// forwarding headers' address) whose requests outrun its token bucket waits
// for tokens, and is then turned away with a 429 and a Retry-After.  A bot's
// bucket holds 10 tokens, refilling one per 5 s, and it can wait 5 s: of 13
// requests at once, 10 go ahead, and the others wait, and (usually) only one
// of those gets the token refilled while they did.
add_task(async function test_Admission() {
  const unique = Math.floor(Math.random() * 0xffff).toString(16);
  const headers = { "Fastly-Client-IP": `2001:db8:${unique}::1`, "User-Agent": "curl/8.5.0" };
  // (Pages that don't exist, which nginx doesn't cache.)
  const page = i => `/searchfox/rev/0000000000000000000000000000000000000000/admission-${unique}-${i}`;
  const responses = await Promise.all([...Array(13).keys()].map(i => fetch(page(i), { headers })));
  const statuses = responses.map(response => response.status);
  const rejected = statuses.filter(status => status == 429).length;
  ok(rejected >= 1 && rejected <= 3, `Some are turned away (${statuses})`);
  const turnedAway = responses.find(response => response.status == 429);
  ok(Number(turnedAway?.headers.get("Retry-After")) >= 1, "and told when to retry");

  const local = await fetch(page("local"), { headers: { "User-Agent": "curl/8.5.0" } });
  isnot(local.status, 429, "Requests without forwarding headers aren't limited");
});
