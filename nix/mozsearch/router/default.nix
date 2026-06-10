{
  stdenvNoCC,
  callPackage,
  lib,
  livegrep,
  python3Packages,
  llvmPackages,
  procps,
}: let
  livegrep-grpc3 = callPackage ./livegrep-grpc3.nix {};
in
  stdenvNoCC.mkDerivation {
    pname = "mozsearch-router";
    version = "unstable";

    src = ../../../router;

    nativeBuildInputs = [
      python3Packages.wrapPython
    ];

    pythonPath = map (dep: dep python3Packages) [livegrep-grpc3];

    installPhase = ''
      mkdir -p "$out/lib/python${python3Packages.python.pythonVersion}/site-packages"
      cp $src/* "$out/lib/python${python3Packages.python.pythonVersion}/site-packages"

      mkdir -p $out/bin
      cp $src/router.py $out/bin/router
      cp $src/codesearch.py $out/bin/codesearchctl

      makeWrapperArgs+=( --prefix PATH : "${lib.makeBinPath [livegrep llvmPackages.bintools procps]}" )
      wrapPythonPrograms
    '';
  }
