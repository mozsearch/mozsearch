{
  lib,
  typescript,
  mkYarnPackage,
  fetchFromGitHub,
}:
mkYarnPackage rec {
  pname = "scip-typescript";
  version = "0.3.14";

  src = fetchFromGitHub {
    owner = "sourcegraph";
    repo = pname;
    rev = "v${version}";
    hash = "sha256-PgwcEEFRNfB4hc9iNhi0ayfvBpF++Hnx84DyWz5tfwE=";
  };

  nativeBuildInputs = [
    typescript
  ];

  buildPhase = ''
    runHook preBuild

    export HOME=$(mktemp -d)
    tsc -b deps/@sourcegraph/scip-typescript

    runHook postBuild
  '';

  postInstall = ''
    mv $out/bin/sourcegraph-scip-typescript $out/bin/scip-typescript
  '';

  postFixup = ''
    chmod +x $(realpath $out/bin/scip-typescript)
  '';
}
