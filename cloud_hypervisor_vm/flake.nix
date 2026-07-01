{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    nixos-generators.url = "github:nix-community/nixos-generators";
  };
  outputs = { self, nixpkgs, nixos-generators }: let
    vm-module = ./configuration.nix;
  in {
    packages.x86_64-linux = {
      # Performance/Nix strategy: Nix store sharing + git from /nix, no SSH socket
      vm-perf = nixos-generators.nixosGenerate {
        system = "x86_64-linux";
        format = "raw";
        modules = [ vm-module ];
      };

      # Secure/Agent strategy: same as perf + SSH agent socket forwarding
      vm-sec = nixos-generators.nixosGenerate {
        system = "x86_64-linux";
        format = "raw";
        modules = [ vm-module ];
      };
    };
  };
}