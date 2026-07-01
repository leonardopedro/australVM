{ config, pkgs, ... }: {
  boot.isContainer = true;
  boot.initrd.availableKernelModules = [ "virtio_net" "virtio_pci" "virtio_fs" ];

  # Mount Host Nix Store via DAX (shared from host's /nix)
  fileSystems."/nix" = {
    device = "host_nix";
    fsType = "virtiofs";
    options = [ "ro" ];
  };

  # Mount SSH agent socket from host (optional — only when virtio-fs tag is provided)
  fileSystems."/run/ssh-agent" = {
    device = "host_ssh";
    fsType = "virtiofs";
    options = [ "ro" ];
  };

  # GPU / Wayland acceleration
  hardware.opengl.enable = true;

  # Define the agent user
  users.users.agent = {
    isNormalUser = true;
    extraGroups = [ "wheel" "video" ];
  };

  # SSH Access for Host-to-VM
  services.openssh.enable = true;
  services.getty.autologinUser = "agent";

  # Systemd settings
  systemd.services.nix-daemon.enable = true;

  # Set SSH_AUTH_SOCK in agent's shell profile if socket mount is present
  environment.etc."profile.d/ssh-agent.sh".text = ''
    if [ -S /run/ssh-agent/ssh-agent.sock ]; then
      export SSH_AUTH_SOCK=/run/ssh-agent/ssh-agent.sock
    fi
  '';
}