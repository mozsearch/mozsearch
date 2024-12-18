{
  rustPlatform,
  fetchFromGitHub,
  cargo-readme,
}:
rustPlatform.buildRustPackage rec {
  pname = "wasm-snip";
  version = "0.5.0";

  src = fetchFromGitHub {
    owner = "mozsearch";
    repo = pname;
    rev = version;
    hash = "sha256-XFC46zoVJIYGtrSTl4rR82355M4MCp1bBV/fmYQzgXQ=";
  };

  cargoLock.lockFile = src + "/Cargo.lock";

  nativeCheckInputs = [cargo-readme];
}
