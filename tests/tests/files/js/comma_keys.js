// A property with a string key with a comma, whose symbols js-analyze escapes
// as "@2C", since commas separate symbols in source records and pages'
// data-symbols attributes.
var commaKeys = {
  "a,b": 1,
};
