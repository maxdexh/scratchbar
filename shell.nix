# TODO: Use a flake with flake-compat, add rust toolchain once 1.93 is in nixpkgs
{pkgs ? import <nixpkgs> {}}:
pkgs.mkShell {
  nativeBuildInputs = [pkgs.pkg-config];

  buildInputs = [
    pkgs.rustPlatform.bindgenHook
    pkgs.wlr-randr

    # For the example controller
    pkgs.pulseaudio
    pkgs.libpulseaudio
  ];
}
