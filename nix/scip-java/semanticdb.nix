{
  stdenv,
  makeWrapper,
  fetchurl,
  jre_headless,
}:
stdenv.mkDerivation {
  name = "scip-java-semanticdb";
  version = "0.12.3";

  src = fetchurl {
    url = "https://github.com/scip-code/scip-java/releases/download/v0.12.3/scip-java-v0.12.3";
    hash = "sha256-dSF4TYxL9S89+Vg0tfo56J0nmsMhkht7ePWmBFtwm/4=";
    executable = true;
  };

  dontUnpack = true;

  nativeBuildInputs = [
    makeWrapper
  ];

  installPhase = ''
    makeWrapper $src $out/bin/scip-java-semanticdb --set JAVA_HOME ${jre_headless}
  '';
}
