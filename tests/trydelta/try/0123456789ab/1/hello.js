// Say hello to someone.
function welcomePerson(name) {
  return "Hello, " + name;
}

// Say hello to everyone.
function greetEveryone(names) {
  return names.map(name => welcomePerson(name)).join("\n");
}
