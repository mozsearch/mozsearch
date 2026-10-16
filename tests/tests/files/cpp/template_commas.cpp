// Symbols with commas (ex: between a template's arguments in a partial
// specialization's members' qualified names), which the clang plugin and
// merge-analyses escape as "@2C", since commas separate symbols in source
// records and pages' data-symbols attributes.
namespace template_commas {

template <typename A, typename B>
struct Pair {};

template <typename A>
struct Pair<A, int> {
  using First = A;
  First mFirst;
};

Pair<char, int>::First gFirst = 'a';

}  // namespace template_commas
