Vagrant.configure("2") do |config|
  config.vm.box = "generic/ubuntu1804"
  config.vm.box_version = "2.0.6"

  config.vm.provision :shell, privileged: false, path: "infrastructure/indexer-provision.sh"
  config.vm.provision :shell, privileged: false, path: "infrastructure/vagrant/indexer-provision.sh"
  config.vm.provision :shell, privileged: false, path: "infrastructure/web-server-provision.sh"

  config.vm.network :forwarded_port, guest: 80, host: 8001

  config.vm.provider "virtualbox" do |v|
    v.memory = 10000
    v.cpus = 4
  end

  config.vm.provider "libvirt" do |v, override|
    # Need to do this manually for libvirt...
    override.vm.synced_folder './', '/vagrant', type: 'nfs', nfs_udp: false, accessmode: "squash"

    v.memory = 10000
    v.cpus = 4
  end
end
