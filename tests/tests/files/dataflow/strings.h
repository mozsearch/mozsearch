// A stand-in for Gecko's string classes (xpcom/string), whose methods are
// only declared here, so that the dataflow analysis can't see into them, as it
// can't see into Gecko's: traces step over their calls by the hints in
// config_defaults/ontology-mapping.toml (nsTSubstring::Assign's,
// nsTString::nsTString's).

template <typename T>
class nsTSubstring {
 public:
  nsTSubstring();
  void Assign(const nsTSubstring<T>& aOther);
  int Length() const;
};

template <typename T>
class nsTString : public nsTSubstring<T> {
 public:
  nsTString();
  explicit nsTString(const nsTSubstring<T>& aOther);
};
