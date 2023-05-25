{ config, lib, pkgs, ... }:
{
  system.stateVersion = "22.11";

  # See https://github.com/nix-community/nixos-generators/issues/192
  boot.initrd.kernelModules = [
    "virtio_blk"
    "virtio_pmem"
    "virtio_console"
    "virtio_pci"
    "virtio_mmio"
  ];

  environment.systemPackages = [
    pkgs.util-linux
  ];

  services = {
    getty.autologinUser = lib.mkDefault "root";
    qemuGuest.enable = true;
  };
}
