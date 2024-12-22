{
  inputs = {
    nixpkgs.url = github:NixOS/nixpkgs/nixpkgs-unstable;
    flake-utils.url = github:numtide/flake-utils;
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  nixConfig = {
    extra-substituters = ["https://nix-community.cachix.org"];
    extra-trusted-public-keys = ["nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="];
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    fenix,
  }: (
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = nixpkgs.legacyPackages.${system}.extend fenix.overlays.default;

        rustToolchain = pkgs.fenix.stable.toolchain;

        # A symlink from `docker` to `podman`, because the scripts call `docker`.
        dockerCompat = pkgs.runCommandNoCC "docker-podman-compat" {} ''
          mkdir -p $out/bin
          ln -s ${pkgs.podman}/bin/podman $out/bin/docker
        '';

        pythonPackages = p:
          with p; [
            boto3
            rich
          ];
      in {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            dockerCompat
            podman

            # Those are probably not all required, copied from
            # https://gist.github.com/adisbladis/187204cb772800489ee3dac4acdd9947
            runc # Container runtime
            conmon # Container runtime monitor
            skopeo # Interact with container registry
            slirp4netns # User-mode networking for unprivileged namespaces
            fuse-overlayfs # CoW for images, much faster than default vfs

            jq

            (python3.withPackages pythonPackages)
            awscli2

            # Dependencies required to build tools
            rustToolchain
            openssl
            cmake
            pkg-config

            # Must be before (unwrapped) clang in path
            clang-tools_19

            # Dependencies required to build clang-plugin
            clang_19
            llvmPackages_19.libllvm
            llvmPackages_19.libclang

            gdb

            scip
            protobuf

            pre-commit
          ];

          PODMAN_USERNS = "keep-id";

          AWS_PROFILE = "searchfox";

          shellHook = ''
            echo "TL;DR:"
            echo "- './build-docker.sh' to build the container"
            echo "- './run-docker.sh' to enter the container"
            echo "- 'cd /vagrant; make build-test-repo' inside to build"
            echo "- open http://localhost:16995 in a browser"
          '';
        };

        formatter = pkgs.alejandra;
      }
    )
  );
}
