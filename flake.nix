{
  inputs = {
    nixpkgs.url = github:NixOS/nixpkgs;
    flake-utils.url = github:numtide/flake-utils;
  };

  outputs = { self, nixpkgs, flake-utils }: (
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        # A symlink from `docker` to `podman`, because the scripts call `docker`.
        dockerCompat = pkgs.runCommandNoCC "docker-podman-compat" {} ''
          mkdir -p $out/bin
          ln -s ${pkgs.podman}/bin/podman $out/bin/docker
        '';

        pythonPackages = p: with p; [
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
            runc  # Container runtime
            conmon  # Container runtime monitor
            skopeo  # Interact with container registry
            slirp4netns  # User-mode networking for unprivileged namespaces
            fuse-overlayfs  # CoW for images, much faster than default vfs

            jq

            awscli2
            (python3.withPackages pythonPackages)
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
      }
    )
  );
}
