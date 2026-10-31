{pkgs ? import <nixpkgs> {}}: let
  inherit (pkgs) lib;

  pname = "scratchbar";
  version = "0.1.0";

  rustPackage = pkgs.rustPlatform.buildRustPackage {
    inherit pname version;

    src = lib.cleanSource ./.;

    cargoLock = {
      lockFile = ./Cargo.lock;
      outputHashes = {
        "crossterm-0.29.0" = "sha256-ESJW1fUC62yPjj40Lt3S9sN/cBcuDm0mE1s+VH6jVO4=";
      };
    };

    nativeBuildInputs = with pkgs; [
      pkg-config
      rustPlatform.bindgenHook
    ];

    buildInputs = with pkgs; [
      libpulseaudio
    ];
  };
in pkgs.writeShellApplication {
  name = pname;

  inheritPath = false;

  runtimeInputs = with pkgs; [
    kitty
    wlr-randr
    pulseaudio
  ];

  text = "exec ${rustPackage}/bin/${pname} \"$@\"";

  meta = {
    mainProgram = pname;
  };
}
