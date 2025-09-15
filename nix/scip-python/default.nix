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
  version = "0.6.0-mozsearch";

  src = fetchFromGitHub {
    owner = "asutherland";
    repo = pname;
    rev = "1df2d02e3a72a39007bb6e2c5571ecfa139b8163";
    hash = "sha256-zZGehzemaohtyykHFhc63wxZofKsOFLhOvpOsjSQcUM=";
  };

  nativeBuildInputs = [
    python3
    pkg-config
  ];

  buildInputs = [
    libsecret
  ];

  patches = [
    ./0001-Use-npm-workspace.patch
    ./0002-npm-update.patch
  ];

  npmDepsHash = "sha256-hViAPKYXbFhmWsy6Cj/YhCRKky93WcPvpURY7sr6pdM=";

  npmWorkspace = "packages/pyright-scip";

  preFixup = ''
    cp -R packages $out/lib/node_modules/pyright-root/packages
  '';
}
