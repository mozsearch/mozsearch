// Say hello to someone.
function greetPerson(name) {
  return "Hello, " + name;
}

// Say hello to everyone.
function greetEveryone(names) {
  return names.map(name => greetPerson(name)).join("\n");
}
