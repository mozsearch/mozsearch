{
  self-with-dotgit,
  stdenv,
  rustToolchain,
  git,
  git-cinnabar,
  build-index,
  mozsearch-tools,
  makeWrapper,
}:
stdenv.mkDerivation {
  pname = "searchfox-repo";
  version = "unstable";

  src = self-with-dotgit;

  postPatch = ''
    patchShebangs tests/searchfox/build tests/searchfox/setup
    git add tests/searchfox/build tests/searchfox/setup
    git -c user.name=Nix -c user.email=nix@localhost commit -m "Patch shebangs" --no-verify
  '';

  nativeBuildInputs = [
    # For Rust tests
    rustToolchain

    build-index
    mozsearch-tools

    git
    git-cinnabar
  ];

  buildPhase = ''
    runHook prebuild

    export LC_ALL=C.UTF-8

    export MOZSEARCH_SOURCE_PATH=$(pwd)

    build-index tests searchfox-config.json index

    runHook postBuild
  '';

  dontFixup = true;

  installPhase = ''
    runHook preInstall

    mkdir -p $out
    cp -RL index $out
    substituteInPlace $out/index/config.json --replace-fail $(pwd) $out

    runHook postInstall
  '';
}
