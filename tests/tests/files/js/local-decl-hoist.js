function call_definedLater() {
  definedLater();

  function definedLater() {
    // This should be called.
    console.log("1");
  }
}

function call_definedInBlock() {
  if (true) {
    function definedInBlock() {
      // This should be called.
      console.log("2");
    }
  }

  definedInBlock();
}

function call_definedInBlock_aliasingVar() {
  var aliasingVar = 0;

  if (true) {
    function aliasingVar() {
      // This should be called.
      console.log("3");
    }
  }

  aliasingVar();
}

function call_definedInBlock_aliasingLexical() {
  let aliasingLexical = 0;

  if (true) {
    function aliasingLexical() {
      // This should NOT be called.
      console.log("4");
    }
  }

  aliasingLexical();
}

function call_definedInLaterBlock() {
  definedInLaterBlock();

  if (true) {
    function definedInLaterBlock() {
      // This should NOT be called.
      console.log("5");
    }
  }
}

try {
  call_definedLater();
} catch {}
try {
  call_definedInBlock();
} catch {}
try {
  call_definedInBlock_aliasingVar();
} catch {}
try {
  call_definedInBlock_aliasingLexical();
} catch {}
try {
  call_definedInLaterBlock();
} catch {}
