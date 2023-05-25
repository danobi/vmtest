{ config, lib, pkgs, ... }: rec {
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
    pkgs.bash
    pkgs.util-linux
  ];

  services = {
    getty.autologinUser = lib.mkDefault "root";
    qemuGuest.enable = true;
  };

  # This is necessary to correctly set qemu-guest-agent's PATH
  # to contain all the packages we installed.
  systemd.services.qemu-guest-agent.path = environment.systemPackages;
}
