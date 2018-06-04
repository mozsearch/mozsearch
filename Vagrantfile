Vagrant.configure("2") do |config|
  # We use this image to get a large (160GB) disk rather than the normal 40GB.
  config.vm.box = "cbednarski/ubuntu-1604-large"
  config.vm.box_version = "0.1.0"

  config.vm.provision :shell, privileged: false, path: "infrastructure/indexer-provision.sh"
  config.vm.provision :shell, privileged: false, path: "infrastructure/vagrant/indexer-provision.sh"
  config.vm.provision :shell, privileged: false, path: "infrastructure/web-server-provision.sh"

  config.vm.network :forwarded_port, guest: 80, host: 8000

  config.vm.provider "virtualbox" do |v|
    v.memory = 10000
    v.cpus = 4
  end
end
