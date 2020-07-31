Vagrant.configure("2") do |config|
  config.vm.box = "generic/ubuntu1804"
  config.vm.box_version = "2.0.6"

  config.vm.provision :shell, privileged: false, path: "infrastructure/vagrant/indexer-provision.sh"
  config.vm.provision :shell, privileged: false, path: "infrastructure/indexer-provision.sh"
  config.vm.provision :shell, privileged: false, path: "infrastructure/web-server-provision.sh"

  config.vm.network :forwarded_port, guest: 80, host: 16995

  config.vm.provider "virtualbox" do |v, override|
    override.vm.synced_folder './', '/vagrant'

    v.memory = 10000
    v.cpus = 4
  end

  config.vm.provider "libvirt" do |v, override|
    # Need to do this manually for libvirt...
    # local_lock makes flock() be local to the VM and avoids NFS trying to
    # acquire locks via the NLM sideband protocol.  This is sane unless you
    # are trying to run indexing inside the VM and outside the VM at the same
    # time, which you should not do.
    override.vm.synced_folder './', '/vagrant', type: 'nfs', nfs_udp: false, accessmode: "squash", mount_options: ['local_lock=all']

    v.memory = 10000
    v.cpus = 4
  end
end
