{
  description = "Linux CLI for the Laifen Wave electric toothbrush";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      cargo = builtins.fromTOML (builtins.readFile ./Cargo.toml);
    in
    {
      packages = forAll (pkgs: rec {
        laifen-toolbox = pkgs.rustPlatform.buildRustPackage {
          pname = "laifen-toolbox";
          version = cargo.package.version;
          src = pkgs.lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [ pkgs.installShellFiles ];
          # The tool talks to BlueZ over D-Bus itself: no runtime dependency.
          postInstall = ''
            installShellCompletion --cmd laifen \
              --bash <($out/bin/laifen completions bash) \
              --zsh <($out/bin/laifen completions zsh) \
              --fish <($out/bin/laifen completions fish)
          '';
          meta = {
            description = cargo.package.description;
            homepage = "https://github.com/marc-alexis-com/laifen-toolbox";
            license = pkgs.lib.licenses.mit;
            mainProgram = "laifen";
            platforms = pkgs.lib.platforms.linux;
          };
        };
        default = laifen-toolbox;
      });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          packages = [ pkgs.cargo pkgs.rustc pkgs.rustfmt pkgs.clippy ];
        };
      });
    };
}
