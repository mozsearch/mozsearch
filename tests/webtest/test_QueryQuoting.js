"use strict";

// A query's search box quotes C++ qualified names as they're typed (see
// search.js's QueryQuoting): typing `::` after a name puts a quote before it,
// and shows its closing quote, which a space, Enter or leaving the box writes.

function doc() {
  return frame.contentDocument;
}

// (Type `text` at the caret, as the keyboard does: each character's
// beforeinput, which can be canceled, then its insertion and input.)
function type(input, text) {
  const win = input.ownerDocument.defaultView;
  for (const data of text) {
    const before = new win.InputEvent("beforeinput", {
      bubbles: true, cancelable: true, inputType: "insertText", data,
    });
    if (input.dispatchEvent(before)) {
      input.setRangeText(data, input.selectionStart, input.selectionEnd, "end");
      input.dispatchEvent(new win.InputEvent("input", {
        bubbles: true, inputType: "insertText", data,
      }));
    }
  }
}

function phantom() {
  const quote = doc().querySelector(".query-phantom-quote");
  return quote && !quote.hidden;
}

add_task(async function test_QueryQuoting() {
  await TestUtils.loadPath("/trydelta/source/user.cpp");
  const input = doc().querySelector("#query");

  type(input, "Widget:");
  is(input.value, "Widget:", "A name and a colon aren't quoted");
  type(input, ":Fr");
  is(input.value, "'Widget::Fr", "A qualified name is quoted as it's typed");
  ok(phantom(), "with its closing quote shown");
  type(input, "ob ");
  is(input.value, "'Widget::Frob' ", "which a space writes");
  ok(!phantom(), "and no longer shows");

  input.value = "";
  type(input, "calls-to:Widget::Frob depth:2");
  is(input.value, "calls-to:'Widget::Frob' depth:2", "A term's value is quoted");

  input.value = "";
  type(input, "Pair<A, B>::Get x");
  is(input.value, "Pair<A, B>::Get x", "(A name with spaces in its template's arguments isn't)");

  input.value = "";
  type(input, "Foo<A,B>::Get x");
  is(input.value, "'Foo<A,B>::Get' x", "but one without them is");

  input.value = "";
  type(input, "'Widget::Frob'");
  is(input.value, "'Widget::Frob'", "A quoted name isn't quoted again");
  input.value = "";
  type(input, "Widget::Frob'");
  is(input.value, "'Widget::Frob'", "and a quote typed closes one");
  ok(!phantom(), "with nothing more shown");

  input.value = "";
  type(input, "Widget::Frob");
  // (As if focus moved, which a frame's document may not have.)
  input.dispatchEvent(new (doc().defaultView.FocusEvent)("blur"));
  is(input.value, "'Widget::Frob'", "Leaving the box closes the name");

  input.value = "";
  type(input, "Widget::Frob");
  const loaded = TestUtils.waitForLoad();
  input.form.requestSubmit();
  await loaded;
  is(new URLSearchParams(doc().location.search).get("q"), "'Widget::Frob'",
     "and so does searching");
  ok(!new URLSearchParams(doc().location.search).has("input"),
     "which is the query as typed");
});
