{
  lib,
  buildNpmPackage,
  python3,
  pkg-config,
  libsecret,
  fetchFromGitHub,
}:
buildNpmPackage rec {
  pname = "scip-python";
  version = "0.6.6";

  src = fetchFromGitHub {
    owner = "sourcegraph";
    repo = pname;
    rev = "v${version}";
    hash = "sha256-gJwoOD43o7bMLrPzeshrYb5A4nt0SHgX7ZNwAQ1hqXc=";
  };

  nativeBuildInputs = [
    python3
    pkg-config
  ];

  buildInputs = [
    libsecret
  ];

  patches = [
    ./0001-Use-npm-workspaces.patch
    ./0002-npm-run-fix-syncpack.patch
    ./0003-Fix-build.patch
    ./0004-Lambda-arguments-should-be-locals-when-the-lambda-is.patch
    ./0005-Silence-spurious-Could-not-find-package-information-.patch
  ];

  npmDepsHash = "sha256-PAHhFWfxx8NfDUbplHv93bWH+06zYhReMKR7V2YlJFA=";

  npmWorkspace = "packages/pyright-scip";

  preFixup = ''
    cp -R packages $out/lib/node_modules/pyright-root/packages
  '';
}
