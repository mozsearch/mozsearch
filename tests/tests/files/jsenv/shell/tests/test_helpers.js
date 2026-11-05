// A JS shell test, run after its directories' shell.js files.
loadRelativeToScript("../lib/near.js");
assertSame(1, 1);
assertAbout(1.1, 1);
assertNear(1, 1);
throw new Error("done");
