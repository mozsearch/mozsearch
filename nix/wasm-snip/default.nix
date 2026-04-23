{
  rustPlatform,
  fetchFromGitHub,
  cargo-readme,
}:
rustPlatform.buildRustPackage rec {
  pname = "wasm-snip";
  version = "0.4.0";

  src = fetchFromGitHub {
    owner = "rustwasm";
    repo = pname;
    rev = version;
    hash = "sha256-qLqelbHYIyDhTDULfJjFQ8UlaQwxqvHDOJ8F9lZs2m8=";
  };

  postPatch = ''
    ln -s ${./Cargo.lock} Cargo.lock
  '';
  cargoLock.lockFile = ./Cargo.lock;

  nativeCheckInputs = [cargo-readme];
}
