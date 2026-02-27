{
  llvmPackages,
}:
llvmPackages.stdenv.mkDerivation {
  pname = "mozsearch-clang-plugin";
  version = "unstable";

  src = ../../clang-plugin;

  buildInputs = [
    llvmPackages.libllvm
    llvmPackages.libclang
  ];

  # Give users access to the matching clang and llvm versions
  passthru = {
    inherit llvmPackages;
  };

  installPhase = ''
    runHook preInstall

    install -Dm644 libclang-index-plugin.so $out/lib/libclang-index-plugin.so

    runHook postInstall
  '';
}
