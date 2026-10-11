# Bikeshed (https://github.com/speced/bikeshed), from mozsearch's fork, for its
# `bikeshed index`, which spec-analyze runs on Bikeshed specs' sources (see
# tools/src/spec_analyze/bikeshed.rs): nixpkgs' package with the fork's source.
{
  bikeshed,
  fetchFromGitHub,
}:

bikeshed.overridePythonAttrs (old: {
  version = "7.1.3-mozsearch-index";
  src = fetchFromGitHub {
    owner = "mozsearch";
    repo = "bikeshed";
    # The `mozsearch-index` branch.
    rev = "98af0480c794baedf7eb66a4882560aaf317c0a6";
    hash = "sha256-gpdGfjx8MbYxQ+RbPG00asJ7tmHAkWhbRCHNtlwTJuc=";
  };
  # (nixpkgs' patch removes a check of the installation which Bikeshed no
  # longer has.)
  patches = [ ];
  # (Bikeshed no longer uses json-home-client, which nixpkgs marks broken.)
  dependencies = builtins.filter (dep: (dep.pname or "") != "json-home-client") old.dependencies;
  # (Its tests build its test specs and compare their output, which the
  # fork's changes don't change; the import check remains.)
  doCheck = false;
})
